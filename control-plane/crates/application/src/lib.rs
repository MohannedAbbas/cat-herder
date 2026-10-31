//! Application services: orchestrate the domain ports into use cases.
//!
//! Generic over the port implementations (not `dyn`), per `PLAN.md` §5.

pub mod metrics;

use std::collections::HashSet;
use std::time::SystemTime;

use cat_herder_domain::api::{
    SessionRun, is_safe_repo_url, new_session_id, repo_identity_from_url,
};
use cat_herder_domain::entities::{
    ApprovalRequest, ApprovalStatus, Budget, Review, ReviewStatus, ReviewVerdict, Run, RunId,
    RunState, Usage,
};
use cat_herder_domain::events::{Event, event_type};
use cat_herder_domain::ports::{
    AgentRuntime, ApprovalRepository, BudgetCounter, BudgetRepository, ConsumeOutcome,
    CredentialBroker, DomainResult, EventBus, EventRepository, ReviewRepository, RunRepository,
    RunRequest, SessionRepository, TransitionRepository,
};
use cat_herder_domain::reconcile::reconcile_run;
use cat_herder_domain::shard::shard_for;
use cat_herder_domain::state::is_terminal;

/// Publish an event to the fan-out bus (best-effort — the durable authority is the
/// transition repository / event log, not the transport).
async fn publish_event<B: EventBus>(bus: &B, event: &Event) {
    if let Err(e) = bus.publish(event).await {
        eprintln!("event publish failed for run {}: {e}", event.run_id.0);
    }
}

/// Persist a single lifecycle transition atomically — the state change and its event
/// land in one transaction (`PLAN.md` §6/§9) — then fan the event out to the transport.
async fn persist_transition<T: TransitionRepository, B: EventBus>(
    run: &mut Run,
    target: RunState,
    transitions: &T,
    bus: &B,
) -> DomainResult<()> {
    run.transition(target)?;
    let event = Event::lifecycle_for(run);
    let (stored_run, stored_event) = transitions.save_transition(run, &event).await?;
    *run = stored_run;
    publish_event(bus, &stored_event).await;
    Ok(())
}

/// Mint a short-lived, read-only clone credential for `run`'s declared repo via the
/// broker, returning the materialized secret for out-of-band injection (`PLAN.md`
/// §11.3 "Clone-scoped credentials"). Returns `None` when the run has no repo or the
/// mint fails — the runtime then falls back to a registered repo token, or none for
/// public repos. The mint never fails the run: a repo without a broker is not an error.
async fn mint_clone_token<C: CredentialBroker>(broker: &C, run: &Run) -> Option<String> {
    let repo = run.repo.as_deref()?;
    let identity = repo_identity_from_url(repo)?;
    match broker.mint(run.id, &identity.id(), "clone", "read").await {
        Ok(minted) => Some(minted.secret),
        Err(e) => {
            eprintln!(
                "clone credential mint failed for run {} ({identity_id}): {e}",
                run.id.0,
                identity_id = identity.id()
            );
            None
        }
    }
}

/// Spawn a run's execution, minting a clone credential first (best-effort).
async fn spawn_run<C: CredentialBroker, R: AgentRuntime>(
    broker: &C,
    runtime: &R,
    run: &Run,
) -> DomainResult<()> {
    let token = mint_clone_token(broker, run).await;
    runtime.spawn(run, token.as_deref()).await
}

/// Revoke every active credential grant for a run (terminal teardown, `PLAN.md`
/// §11.3). Best-effort: revocation is defense-in-depth, since installation tokens lapse
/// via TTL regardless.
async fn revoke_run_credentials<C: CredentialBroker>(broker: &C, run_id: RunId) {
    if let Err(e) = broker.revoke_run(run_id).await {
        eprintln!("credential revocation failed for run {}: {e}", run_id.0);
    }
}

/// Use case: accept a trigger's request, persist it, spawn the agent, and advance the
/// lifecycle state machine through to `PodPending`. The reconciler (`reconcile_once`)
/// advances it to `Running` (and terminal states) once the sandbox is actually ready.
///
/// Each run is auto-linked to a freshly created session (title derived from the
/// task spec).
pub async fn start_run<R, P, S, T, B, C>(
    req: RunRequest,
    runtime: &R,
    repo: &P,
    sessions: &S,
    transitions: &T,
    bus: &B,
    broker: &C,
) -> DomainResult<Run>
where
    R: AgentRuntime,
    P: RunRepository,
    S: SessionRepository,
    T: TransitionRepository,
    B: EventBus,
    C: CredentialBroker,
{
    let run = Run {
        id: RunId(0), // assigned by the repository
        kind: req.kind,
        state: RunState::Queued,
        task_spec: req.task_spec,
        session_id: None,
        max_concurrency: req.max_concurrency,
        created_at: SystemTime::now(),
        trigger: req.trigger,
        actor: req.actor,
        model: req.model,
        image: req.image,
        repo: req.repo.filter(|r| is_safe_repo_url(r)),
        language: req.language,
        workflow_ref: req.workflow_ref.clone(),
        runtime_tier: req.runtime_tier,
        started_at: None,
        finished_at: None,
        pr_url: None,
        report: None,
        input_tokens: 0,
        output_tokens: 0,
        callback_url: req.callback_url.clone(),
    };

    // Create the run and its `run.queued` event atomically (the transition repository
    // assigns the id and writes both).
    let event = Event::lifecycle_for(&run);
    let (mut run, stored_event) = transitions.save_transition(&run, &event).await?;
    publish_event(bus, &stored_event).await;

    // Auto-create a session and link the run to it.
    let session_id = new_session_id();
    sessions
        .create(&session_id, &session_title(&run.task_spec))
        .await?;
    run.session_id = Some(session_id.clone());
    run = repo.save(&run).await?;

    persist_transition(&mut run, RunState::Scheduled, transitions, bus).await?;
    persist_transition(&mut run, RunState::PodPending, transitions, bus).await?;

    // Project the run into the session's display rows (now `PodPending`).
    sessions
        .add_run(&session_id, &SessionRun::from(&run))
        .await?;

    // Mint a short-lived clone credential and spawn (best-effort mint).
    spawn_run(broker, runtime, &run).await?;

    Ok(run) // PodPending — the reconciler advances from here
}

/// A short, human-readable title derived from a task spec (first 80 chars).
fn session_title(task_spec: &str) -> String {
    let title: String = task_spec.trim().chars().take(80).collect();
    if title.is_empty() {
        "Untitled session".to_string()
    } else {
        title
    }
}

/// Reconcile all active runs against their execution status, advancing the lifecycle
/// state machine where possible. Returns the number of runs advanced or recovered.
///
/// When the reconciler itself reaches a final state (e.g. the stuck-run watchdog maps a
/// failed/liveness-killed or expired sandbox to `Stuck`/`TimedOut`), it also reclaims the
/// execution — mirroring the `finished` event path — so a dead sandbox doesn't linger.
pub async fn reconcile_once<R, P, S, T, B, C>(
    runtime: &R,
    repo: &P,
    sessions: &S,
    transitions: &T,
    bus: &B,
    broker: &C,
) -> DomainResult<usize>
where
    R: AgentRuntime,
    P: RunRepository,
    S: SessionRepository,
    T: TransitionRepository,
    B: EventBus,
    C: CredentialBroker,
{
    reconcile_runs(
        repo.list_all().await?,
        runtime,
        sessions,
        transitions,
        bus,
        broker,
    )
    .await
}

/// Reconcile the runs owned by `shards` (of `shard_count`) — the sharded-reconciler
/// entry point (`PLAN.md` §6). Runs are fetched **once** and filtered to the set of
/// owned shards, so a leader reconciles all its shards from a single read regardless
/// of how many it holds. Only runs whose `run_id` hashes to an owned shard are
/// advanced/recovered.
#[allow(clippy::too_many_arguments)]
pub async fn reconcile_shards<R, P, S, T, B, C>(
    shards: &HashSet<u32>,
    shard_count: u32,
    runtime: &R,
    repo: &P,
    sessions: &S,
    transitions: &T,
    bus: &B,
    broker: &C,
) -> DomainResult<usize>
where
    R: AgentRuntime,
    P: RunRepository,
    S: SessionRepository,
    T: TransitionRepository,
    B: EventBus,
    C: CredentialBroker,
{
    let runs = repo
        .list_all()
        .await?
        .into_iter()
        .filter(|r| shards.contains(&shard_for(r.id, shard_count)))
        .collect();
    reconcile_runs(runs, runtime, sessions, transitions, bus, broker).await
}

/// Core reconcile over an explicit set of runs (the caller already selected the runs
/// this leader is responsible for). Returns the number of runs advanced or recovered.
///
/// For each active run it reads the runtime status and either advances the state
/// machine, or — if the sandbox has gone missing — re-spawns a replacement pod so the
/// run resumes from its last checkpoint (crash recovery, `PLAN.md` §6). The `finished`
/// event remains the only terminal signal, so a missing sandbox never marks a run
/// terminal here.
pub async fn reconcile_runs<R, S, T, B, C>(
    runs: Vec<Run>,
    runtime: &R,
    sessions: &S,
    transitions: &T,
    bus: &B,
    broker: &C,
) -> DomainResult<usize>
where
    R: AgentRuntime,
    S: SessionRepository,
    T: TransitionRepository,
    B: EventBus,
    C: CredentialBroker,
{
    let mut changed = 0;

    for run in runs {
        if !matches!(run.state, RunState::PodPending | RunState::Running) {
            continue;
        }

        let status = runtime.status(run.id).await?;
        match status {
            // Crash recovery: a live run whose sandbox is gone gets a fresh pod. The
            // harness resumes from its latest checkpoint (§6); a run with none starts
            // over. We do not mark it terminal here — `finished` is the only signal
            // that does (a stuck harness surfaces as `Failed`, not `None`).
            None => {
                spawn_run(broker, runtime, &run).await?;
                changed += 1;
            }
            Some(status) => {
                let to_apply = reconcile_run(run.state, Some(status));
                if to_apply.is_empty() {
                    continue;
                }

                let mut updated = run.clone();
                for next in to_apply {
                    // One atomic transition per state change, so the log records the full
                    // walk (e.g. PodPending -> Running -> Succeeded) even on a fast-forward.
                    persist_transition(&mut updated, next, transitions, bus).await?;
                    // Cold-start metric (§16/§23): trigger -> running latency.
                    if next == RunState::Running {
                        let elapsed = SystemTime::now()
                            .duration_since(updated.created_at)
                            .map(|d| d.as_secs_f64())
                            .unwrap_or(0.0);
                        metrics::cold_start().record(elapsed);
                    }
                }
                sessions.sync_run(&updated).await?;
                changed += 1;

                // Reclaim the execution once the run is done (details already persisted).
                let done = updated.state == RunState::Succeeded || is_terminal(updated.state);
                if done {
                    if let Err(e) = runtime.terminate(updated.id).await {
                        eprintln!("failed to terminate sandbox for run {}: {e}", updated.id.0);
                    }
                    revoke_run_credentials(broker, updated.id).await;
                }
            }
        }
    }

    Ok(changed)
}

/// Outcome reported by the harness when an agent completes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Succeeded,
    Failed,
    /// The harness detected its own agent stopped making progress (stuck watchdog).
    Stuck,
}

/// The harness's final output, persisted onto a run when it completes.
#[derive(Debug, Clone, Default)]
pub struct Completion {
    /// The agent's final markdown summary.
    pub report: Option<String>,
    /// LLM token counts reported by the harness.
    pub usage: Option<Usage>,
}

/// Use case: record the harness's completion of a run, advancing it to `Succeeded`
/// or `Failed` (from `Running`, or `PodPending` → `Running` → terminal). Idempotent
/// for runs that already reached a final state. Also syncs the session display row.
#[allow(clippy::too_many_arguments)]
pub async fn complete_run<P, S, T, B, C>(
    run_id: RunId,
    outcome: Outcome,
    completion: Completion,
    repo: &P,
    sessions: &S,
    transitions: &T,
    bus: &B,
    broker: &C,
) -> DomainResult<Run>
where
    P: RunRepository,
    S: SessionRepository,
    T: TransitionRepository,
    B: EventBus,
    C: CredentialBroker,
{
    let Completion { report, usage } = completion;

    let run = repo.get(run_id).await?.ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("run {} not found", run_id.0),
        )
    })?;

    let target = match outcome {
        Outcome::Succeeded => RunState::Succeeded,
        Outcome::Failed => RunState::Failed,
        Outcome::Stuck => RunState::Stuck,
    };

    // Idempotent: don't overwrite a run that already reached a final state. Still
    // persist a late report/usage (e.g. a re-delivered `finished` event).
    if run.state == RunState::Succeeded || is_terminal(run.state) {
        let mut run = run;
        let mut changed = false;
        if let Some(report) = report
            && run.report.as_deref() != Some(report.as_str())
        {
            run.report = Some(report);
            changed = true;
        }
        if let Some(u) = usage {
            run.input_tokens = u.input_tokens;
            run.output_tokens = u.output_tokens;
            changed = true;
        }
        if changed {
            let run = repo.save(&run).await?;
            sessions.sync_run(&run).await?;
            return Ok(run);
        }
        return Ok(run);
    }

    let mut run = run;

    // Apply the harness's final report + usage first, so they land in the same atomic
    // write as the terminal state transition.
    if let Some(report) = report {
        run.report = Some(report);
    }
    if let Some(u) = usage {
        run.input_tokens = u.input_tokens;
        run.output_tokens = u.output_tokens;
    }

    match run.state {
        RunState::Running => {
            persist_transition(&mut run, target, transitions, bus).await?;
            revoke_run_credentials(broker, run.id).await;
        }
        RunState::PodPending => {
            persist_transition(&mut run, RunState::Running, transitions, bus).await?;
            persist_transition(&mut run, target, transitions, bus).await?;
            revoke_run_credentials(broker, run.id).await;
        }
        // queued/scheduled/awaiting-input/etc. — not a runnable state; leave alone.
        _ => return Ok(run),
    }

    sessions.sync_run(&run).await?;
    Ok(run)
}

/// Use case: record a human-in-the-loop interrupt (`PLAN.md` §10). Persists a durable
/// `ApprovalRequest` (with the opaque `resume_token`) and advances the run to
/// `AwaitingInput`. The run is left alone (but the approval is still recorded) if it is
/// in a state that cannot accept an interrupt.
// `#[allow]`: six injected dependencies (approval/event/run/session/transition/bus)
// plus the interrupt data exceed clippy's default arity threshold.
#[allow(clippy::too_many_arguments)]
pub async fn interrupt_run<A, E, P, S, T, B>(
    run_id: RunId,
    resume_token: String,
    payload: serde_json::Value,
    approvals: &A,
    events: &E,
    repo: &P,
    sessions: &S,
    transitions: &T,
    bus: &B,
) -> DomainResult<ApprovalRequest>
where
    A: ApprovalRepository,
    E: EventRepository,
    P: RunRepository,
    S: SessionRepository,
    T: TransitionRepository,
    B: EventBus,
{
    let approval = ApprovalRequest {
        id: uuid::Uuid::new_v4().to_string(),
        run_id,
        resume_token: resume_token.clone(),
        payload: payload.clone(),
        status: ApprovalStatus::Pending,
        response: None,
        created_at: SystemTime::now(),
        responded_at: None,
    };
    let approval = approvals.create(&approval).await?;

    // Record the request in the event log (carries the approval id for the deep link).
    let event = Event::new(
        run_id,
        event_type::APPROVAL_REQUESTED,
        None,
        serde_json::json!({
            "approval_id": approval.id,
            "resume_token": resume_token,
            "payload": payload,
        }),
    );
    let stored = events.append(run_id, &event).await?;
    publish_event(bus, &stored).await;

    // Advance the lifecycle to AwaitingInput (from Running, or PodPending -> Running).
    let Some(run) = repo.get(run_id).await? else {
        return Ok(approval);
    };
    let mut run = run;
    match run.state {
        RunState::Running => {
            persist_transition(&mut run, RunState::AwaitingInput, transitions, bus).await?;
        }
        RunState::PodPending => {
            persist_transition(&mut run, RunState::Running, transitions, bus).await?;
            persist_transition(&mut run, RunState::AwaitingInput, transitions, bus).await?;
        }
        _ => return Ok(approval),
    }
    sessions.sync_run(&run).await?;
    Ok(approval)
}

/// Use case: record a human's decision on an approval (`PLAN.md` §10). Approving resumes
/// the run (`AwaitingInput -> Running`); rejecting cancels it. The decision and response
/// are persisted on the `ApprovalRequest`; an `approval.responded` event is appended.
#[allow(clippy::too_many_arguments)]
pub async fn respond_approval<A, E, P, S, T, B, C>(
    approval_id: &str,
    approved: bool,
    response: Option<String>,
    approvals: &A,
    events: &E,
    repo: &P,
    sessions: &S,
    transitions: &T,
    bus: &B,
    broker: &C,
) -> DomainResult<ApprovalRequest>
where
    A: ApprovalRepository,
    E: EventRepository,
    P: RunRepository,
    S: SessionRepository,
    T: TransitionRepository,
    B: EventBus,
    C: CredentialBroker,
{
    let approval = approvals.get(approval_id).await?.ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("approval {approval_id} not found"),
        )
    })?;
    if approval.status != ApprovalStatus::Pending {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("approval {approval_id} already decided"),
        )
        .into());
    }

    let status = if approved {
        ApprovalStatus::Approved
    } else {
        ApprovalStatus::Rejected
    };
    let decided = approvals
        .decide(approval_id, status, response.clone())
        .await?;

    let event = Event::new(
        approval.run_id,
        event_type::APPROVAL_RESPONDED,
        None,
        serde_json::json!({
            "approval_id": approval.id,
            "status": status,
            "response": response,
        }),
    );
    let stored = events.append(approval.run_id, &event).await?;
    publish_event(bus, &stored).await;

    // Advance the lifecycle: approve -> Running (resume), reject -> Cancelled.
    let run = repo.get(approval.run_id).await?.ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("run {} not found", approval.run_id.0),
        )
    })?;
    let mut run = run;
    let target = if approved {
        RunState::Running
    } else {
        RunState::Cancelled
    };
    if run.state == RunState::AwaitingInput {
        persist_transition(&mut run, target, transitions, bus).await?;
        sessions.sync_run(&run).await?;
        // Rejection cancels the run: tear down its clone credential.
        if !approved {
            revoke_run_credentials(broker, run.id).await;
        }
    }

    Ok(decided)
}

/// Render a review-feedback task spec for a follow-up run (`PLAN.md` §10). Rejection
/// or change requests feed back as a new run whose `task_spec` carries the reviewer's
/// verdict + comments, linked to the original run by the review event.
pub fn review_feedback_task_spec(
    original_task_spec: &str,
    run_id: RunId,
    verdict: ReviewVerdict,
    reviewer: Option<&str>,
    feedback: Option<&str>,
) -> String {
    let verdict_label = match verdict {
        ReviewVerdict::Approved => "approved",
        ReviewVerdict::Rejected => "rejected",
        ReviewVerdict::ChangesRequested => "changes requested",
    };
    let reviewer = reviewer.unwrap_or("a reviewer");
    let mut spec = format!(
        "Address review feedback ({verdict_label}) from {reviewer} on run {}.\n",
        run_id.0
    );
    if let Some(fb) = feedback.map(str::trim).filter(|s| !s.is_empty()) {
        spec.push_str(&format!("Reviewer feedback:\n{fb}\n"));
    }
    spec.push_str(&format!("\nOriginal task:\n{original_task_spec}"));
    spec
}

/// Use case: request a result review (`PLAN.md` §10). Persists a pending `Review`
/// (with the escalation `deadline_at`) and advances a `Succeeded` run to
/// `AwaitingReview`. Returns `Ok(None)` when the run is not in `Succeeded` — e.g. a
/// re-delivered `finished` event whose run is already `AwaitingReview` — so a review
/// is never created twice for the same run.
#[allow(clippy::too_many_arguments)]
pub async fn request_review<R, E, P, S, T, B>(
    run_id: RunId,
    deadline_at: SystemTime,
    reviews: &R,
    events: &E,
    repo: &P,
    sessions: &S,
    transitions: &T,
    bus: &B,
) -> DomainResult<Option<Review>>
where
    R: ReviewRepository,
    E: EventRepository,
    P: RunRepository,
    S: SessionRepository,
    T: TransitionRepository,
    B: EventBus,
{
    // Only a run in `Succeeded` can enter review. Guard before creating the durable
    // review so a duplicate `finished` event is a no-op.
    let run = repo.get(run_id).await?.ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("run {} not found", run_id.0),
        )
    })?;
    if run.state != RunState::Succeeded {
        return Ok(None);
    }

    let review = Review {
        id: uuid::Uuid::new_v4().to_string(),
        run_id,
        status: ReviewStatus::Pending,
        verdict: None,
        reviewer: None,
        feedback: None,
        created_at: SystemTime::now(),
        reviewed_at: None,
        deadline_at,
    };
    let review = reviews.create(&review).await?;

    // Record the request in the event log (carries the review id for the deep link).
    let deadline_ms = deadline_at
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let event = Event::new(
        run_id,
        event_type::REVIEW_REQUESTED,
        None,
        serde_json::json!({
            "review_id": review.id,
            "deadline_at_ms": deadline_ms,
        }),
    );
    let stored = events.append(run_id, &event).await?;
    publish_event(bus, &stored).await;

    let mut run = run;
    persist_transition(&mut run, RunState::AwaitingReview, transitions, bus).await?;
    sessions.sync_run(&run).await?;
    Ok(Some(review))
}

/// Use case: record a reviewer's verdict on a result review (`PLAN.md` §10).
/// `Approved` → `Reviewed`; `Rejected`/`ChangesRequested` → `Rejected`. The verdict,
/// reviewer, and feedback are persisted on the `Review`; a `review.decided` event is
/// appended. Returns the decided review.
#[allow(clippy::too_many_arguments)]
pub async fn decide_review<R, E, P, S, T, B>(
    review_id: &str,
    verdict: ReviewVerdict,
    reviewer: Option<String>,
    feedback: Option<String>,
    reviews: &R,
    events: &E,
    repo: &P,
    sessions: &S,
    transitions: &T,
    bus: &B,
) -> DomainResult<Review>
where
    R: ReviewRepository,
    E: EventRepository,
    P: RunRepository,
    S: SessionRepository,
    T: TransitionRepository,
    B: EventBus,
{
    let review = reviews.get(review_id).await?.ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("review {review_id} not found"),
        )
    })?;
    if review.status != ReviewStatus::Pending {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("review {review_id} already decided"),
        )
        .into());
    }

    let decided = reviews
        .decide(review_id, verdict, reviewer.clone(), feedback.clone())
        .await?;

    let event = Event::new(
        review.run_id,
        event_type::REVIEW_DECIDED,
        None,
        serde_json::json!({
            "review_id": review.id,
            "verdict": verdict,
            "reviewer": reviewer,
            "feedback": feedback,
        }),
    );
    let stored = events.append(review.run_id, &event).await?;
    publish_event(bus, &stored).await;

    // Advance the lifecycle: Approved -> Reviewed; otherwise -> Rejected.
    let run = repo.get(review.run_id).await?.ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("run {} not found", review.run_id.0),
        )
    })?;
    let mut run = run;
    let target = match verdict {
        ReviewVerdict::Approved => RunState::Reviewed,
        ReviewVerdict::Rejected | ReviewVerdict::ChangesRequested => RunState::Rejected,
    };
    if run.state == RunState::AwaitingReview {
        persist_transition(&mut run, target, transitions, bus).await?;
        sessions.sync_run(&run).await?;
    }

    Ok(decided)
}

/// Issue a run's budget lease at start (`PLAN.md` §12): persist the durable cap and
/// zero the atomic counter so the metering proxy enforces it from the first model call.
/// A run with no cap (`None`/`0`) is unbounded — a no-op here.
pub async fn issue_budget<BD: BudgetRepository, CT: BudgetCounter>(
    budgets: &BD,
    counter: &CT,
    run_id: RunId,
    cap: Option<u64>,
) -> DomainResult<()> {
    let Some(cap) = cap.filter(|c| *c > 0) else {
        return Ok(());
    };
    budgets
        .save(&Budget {
            run_id,
            cap,
            updated_at: SystemTime::now(),
        })
        .await?;
    counter.reset(run_id).await?;
    Ok(())
}

/// Enforce a run's budget on one model call (`PLAN.md` §12) — the authoritative hard
/// stop the metering proxy calls before each call. Atomically consumes `tokens` against
/// the run's cap; on `Exceeded` the run transitions to `BudgetPaused` and a
/// `budget.exceeded` event is emitted (the notification is the event + deep link, as
/// with §10). Returns the consumption outcome.
#[allow(clippy::too_many_arguments)]
pub async fn consume_budget<CT, BD, P, S, T, B, E>(
    run_id: RunId,
    tokens: u64,
    counter: &CT,
    budgets: &BD,
    repo: &P,
    sessions: &S,
    transitions: &T,
    bus: &B,
    events: &E,
) -> DomainResult<ConsumeOutcome>
where
    CT: BudgetCounter,
    BD: BudgetRepository,
    P: RunRepository,
    S: SessionRepository,
    T: TransitionRepository,
    B: EventBus,
    E: EventRepository,
{
    let Some(budget) = budgets.get(run_id).await? else {
        return Ok(ConsumeOutcome::Ok { consumed: 0 });
    };
    let cap = budget.cap;

    let outcome = counter.consume(run_id, cap, tokens).await?;

    if let ConsumeOutcome::Exceeded { cap, consumed } = outcome {
        // Record the audited budget event (never a secret; attribution data).
        let event = Event::budget_exceeded(run_id, cap, consumed);
        let stored = events.append(run_id, &event).await?;
        publish_event(bus, &stored).await;

        // Pause the run: only a live `Running` run can be paused by the budget.
        if let Some(run) = repo.get(run_id).await?
            && run.state == RunState::Running
        {
            let mut run = run;
            persist_transition(&mut run, RunState::BudgetPaused, transitions, bus).await?;
            sessions.sync_run(&run).await?;
        }
    }

    Ok(outcome)
}

/// Raise (or set) a run's budget cap and resume a paused run (`PLAN.md` §12). Updating
/// the durable cap and zeroing the counter lets a `BudgetPaused` run continue;
/// otherwise the new cap applies from the next model call. Returns the updated run.
#[allow(clippy::too_many_arguments)]
pub async fn raise_budget<BD, CT, P, S, T, B>(
    run_id: RunId,
    cap: u64,
    budgets: &BD,
    counter: &CT,
    repo: &P,
    sessions: &S,
    transitions: &T,
    bus: &B,
) -> DomainResult<Run>
where
    BD: BudgetRepository,
    CT: BudgetCounter,
    P: RunRepository,
    S: SessionRepository,
    T: TransitionRepository,
    B: EventBus,
{
    let run = repo.get(run_id).await?.ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("run {} not found", run_id.0),
        )
    })?;

    budgets
        .save(&Budget {
            run_id,
            cap,
            updated_at: SystemTime::now(),
        })
        .await?;
    counter.reset(run_id).await?;

    // Resume a budget-paused run back to Running.
    let mut run = run;
    if run.state == RunState::BudgetPaused {
        persist_transition(&mut run, RunState::Running, transitions, bus).await?;
        sessions.sync_run(&run).await?;
    }
    Ok(run)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cat_herder_domain::entities::RunKind;
    use cat_herder_domain::ports::EventRepository;
    use cat_herder_domain::reconcile::RuntimeStatus;
    use cat_herder_store_inmemory::{
        InMemoryApprovalRepository, InMemoryBudgetCounter, InMemoryBudgetRepository,
        InMemoryEventRepository, InMemoryReviewRepository, InMemoryRunRepository,
        InMemorySessionRepository, InMemoryTransitionRepository,
    };

    struct StubRuntime {
        status: Option<RuntimeStatus>,
        spawns: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        last_token: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    }

    impl StubRuntime {
        fn new(status: Option<RuntimeStatus>) -> Self {
            Self {
                status,
                spawns: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                last_token: std::sync::Arc::new(std::sync::Mutex::new(None)),
            }
        }

        /// How many times `spawn` has been called (crash-recovery / re-spawn).
        fn spawn_count(&self) -> usize {
            self.spawns.load(std::sync::atomic::Ordering::Relaxed)
        }

        /// The git token most recently passed to `spawn` (the broker-minted secret).
        fn last_token(&self) -> Option<String> {
            self.last_token.lock().unwrap().clone()
        }
    }

    impl AgentRuntime for StubRuntime {
        async fn spawn(&self, _run: &Run, git_token: Option<&str>) -> DomainResult<()> {
            self.spawns
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            *self.last_token.lock().unwrap() = git_token.map(str::to_owned);
            Ok(())
        }
        async fn status(&self, _id: RunId) -> DomainResult<Option<RuntimeStatus>> {
            Ok(self.status)
        }
        async fn signal(&self, _id: RunId, _state: RunState) -> DomainResult<()> {
            Ok(())
        }
        async fn terminate(&self, _id: RunId) -> DomainResult<()> {
            Ok(())
        }
    }

    /// No-op fan-out bus for tests.
    struct StubBus;

    impl EventBus for StubBus {
        async fn publish(&self, _event: &Event) -> DomainResult<()> {
            Ok(())
        }
    }

    /// No-op credential broker for tests: mint always fails (no broker configured), so
    /// `spawn` falls back to `None` — matching the no-broker path the tests exercise.
    struct StubBroker;

    impl CredentialBroker for StubBroker {
        async fn mint(
            &self,
            _run: RunId,
            _target: &str,
            _action: &str,
            _scope: &str,
        ) -> DomainResult<cat_herder_domain::ports::MintedCredential> {
            Err("no broker configured".into())
        }
        async fn revoke(&self, _run: RunId, _grant_id: &str) -> DomainResult<()> {
            Ok(())
        }
        async fn revoke_run(&self, _run: RunId) -> DomainResult<()> {
            Ok(())
        }
    }

    /// Recording broker for the clone-path wiring tests: mint returns a known secret
    /// and records the `(run_id, target)`; `revoke_run` records the run id so tests can
    /// assert teardown happened.
    #[derive(Default)]
    struct RecordingBroker {
        minted: std::sync::Mutex<Vec<(RunId, String)>>,
        revoked_runs: std::sync::Mutex<Vec<RunId>>,
    }

    impl RecordingBroker {
        fn minted(&self) -> Vec<(RunId, String)> {
            self.minted.lock().unwrap().clone()
        }
        fn revoked_runs(&self) -> Vec<RunId> {
            self.revoked_runs.lock().unwrap().clone()
        }
    }

    impl CredentialBroker for RecordingBroker {
        async fn mint(
            &self,
            run: RunId,
            target: &str,
            action: &str,
            scope: &str,
        ) -> DomainResult<cat_herder_domain::ports::MintedCredential> {
            self.minted.lock().unwrap().push((run, target.to_string()));
            let grant = cat_herder_domain::entities::CredentialGrant {
                id: format!("cred-{}", self.minted.lock().unwrap().len()),
                run_id: run,
                target: target.to_string(),
                action: action.to_string(),
                scope: scope.to_string(),
                status: cat_herder_domain::entities::CredentialGrantStatus::Active,
                expires_at: std::time::SystemTime::now() + std::time::Duration::from_secs(3600),
                created_at: std::time::SystemTime::now(),
                revoked_at: None,
                token_id: None,
            };
            Ok(cat_herder_domain::ports::MintedCredential {
                grant,
                secret: "ghs_fake_token".into(),
            })
        }
        async fn revoke(&self, _run: RunId, _grant_id: &str) -> DomainResult<()> {
            Ok(())
        }
        async fn revoke_run(&self, run: RunId) -> DomainResult<()> {
            self.revoked_runs.lock().unwrap().push(run);
            Ok(())
        }
    }

    /// A [`RunRequest`] with the repo field set (for clone-path tests).
    fn req_with_repo(repo: &str) -> RunRequest {
        RunRequest {
            kind: RunKind::Agent,
            task_spec: "test".into(),
            dedup_key: String::new(),
            max_concurrency: 1,
            trigger: "api".into(),
            actor: None,
            model: None,
            image: None,
            repo: Some(repo.to_string()),
            language: None,
            workflow_ref: None,
            runtime_tier: Default::default(),
            budget: None,
            callback_url: None,
        }
    }

    fn req() -> RunRequest {
        RunRequest {
            kind: RunKind::Agent,
            task_spec: "test".into(),
            dedup_key: String::new(),
            max_concurrency: 1,
            trigger: "api".into(),
            actor: None,
            model: None,
            image: None,
            repo: None,
            language: None,
            workflow_ref: None,
            runtime_tier: Default::default(),
            budget: None,
            callback_url: None,
        }
    }

    #[tokio::test]
    async fn start_run_advances_to_pod_pending_and_persists() {
        let repo = InMemoryRunRepository::new();
        let sessions = InMemorySessionRepository::default();
        let events = InMemoryEventRepository::default();
        let transitions = InMemoryTransitionRepository::new(repo.clone(), events.clone());
        let run = start_run(
            req(),
            &StubRuntime::new(None),
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &StubBroker,
        )
        .await
        .unwrap();

        assert_eq!(run.state, RunState::PodPending);
        assert_ne!(run.id, RunId(0));

        let stored = repo.get(run.id).await.unwrap().unwrap();
        assert_eq!(stored.state, RunState::PodPending);
    }

    #[tokio::test]
    async fn start_run_copies_callback_url_onto_run() {
        let repo = InMemoryRunRepository::new();
        let sessions = InMemorySessionRepository::default();
        let events = InMemoryEventRepository::default();
        let transitions = InMemoryTransitionRepository::new(repo.clone(), events.clone());
        let mut request = req();
        request.callback_url = Some("https://example.com/cb".into());
        let run = start_run(
            request,
            &StubRuntime::new(None),
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &StubBroker,
        )
        .await
        .unwrap();

        assert_eq!(run.callback_url.as_deref(), Some("https://example.com/cb"));
        let stored = repo.get(run.id).await.unwrap().unwrap();
        assert_eq!(stored.callback_url.as_deref(), Some("https://example.com/cb"));
    }

    #[tokio::test]
    async fn start_run_emits_ordered_lifecycle_events() {
        let repo = InMemoryRunRepository::new();
        let sessions = InMemorySessionRepository::default();
        let events = InMemoryEventRepository::default();
        let transitions = InMemoryTransitionRepository::new(repo.clone(), events.clone());
        let run = start_run(
            req(),
            &StubRuntime::new(None),
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &StubBroker,
        )
        .await
        .unwrap();

        let evs = events.list(run.id, 0, 100).await.unwrap();
        let types: Vec<&str> = evs.iter().map(|e| e.event_type.as_str()).collect();
        assert_eq!(types, ["run.queued", "run.scheduled", "run.pod_pending"]);
        // seq is assigned monotonic from 1.
        assert_eq!(evs.iter().map(|e| e.seq).collect::<Vec<_>>(), [1, 2, 3]);
    }

    #[tokio::test]
    async fn start_run_creates_and_links_session() {
        let repo = InMemoryRunRepository::new();
        let sessions = InMemorySessionRepository::default();
        let events = InMemoryEventRepository::default();
        let transitions = InMemoryTransitionRepository::new(repo.clone(), events.clone());
        let run = start_run(
            req(),
            &StubRuntime::new(None),
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &StubBroker,
        )
        .await
        .unwrap();

        // The run is linked to a session, and that session has the run's display row
        // (projected at the final pre-reconcile state, PodPending).
        let session_id = run.session_id.as_deref().expect("run linked to a session");
        let session = sessions
            .get(session_id)
            .await
            .unwrap()
            .expect("session exists");
        assert_eq!(session.runs.len(), 1);
        assert_eq!(session.runs[0].id, format!("run-{}", run.id.0));
        assert_eq!(session.runs[0].task_spec, "test");
        assert_eq!(session.runs[0].state, RunState::PodPending);
    }

    #[tokio::test]
    async fn reconcile_walks_pod_pending_to_succeeded() {
        let repo = InMemoryRunRepository::new();
        let sessions = InMemorySessionRepository::default();
        let events = InMemoryEventRepository::default();
        let transitions = InMemoryTransitionRepository::new(repo.clone(), events.clone());
        let run = start_run(
            req(),
            &StubRuntime::new(None),
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &StubBroker,
        )
        .await
        .unwrap();
        assert_eq!(run.state, RunState::PodPending);
        let session_id = run.session_id.clone().unwrap();

        // ready -> running (display row synced)
        let n = reconcile_once(
            &StubRuntime::new(Some(RuntimeStatus::Ready)),
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &StubBroker,
        )
        .await
        .unwrap();
        assert_eq!(n, 1);
        assert_eq!(
            repo.get(run.id).await.unwrap().unwrap().state,
            RunState::Running
        );
        assert_eq!(
            sessions.get(&session_id).await.unwrap().unwrap().runs[0].state,
            RunState::Running
        );

        // gone -> succeeded is no longer the fallback: a missing sandbox re-spawns.
        // Here the sandbox reports a clean terminal success, which advances Running
        // -> Succeeded (display row synced).
        let n = reconcile_once(
            &StubRuntime::new(Some(RuntimeStatus::Succeeded)),
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &StubBroker,
        )
        .await
        .unwrap();
        assert_eq!(n, 1);
        assert_eq!(
            repo.get(run.id).await.unwrap().unwrap().state,
            RunState::Succeeded
        );
        assert_eq!(
            sessions.get(&session_id).await.unwrap().unwrap().runs[0].state,
            RunState::Succeeded
        );

        // The full lifecycle walk is in the log, in order.
        let evs = events.list(run.id, 0, 100).await.unwrap();
        let types: Vec<&str> = evs.iter().map(|e| e.event_type.as_str()).collect();
        assert_eq!(
            types,
            [
                "run.queued",
                "run.scheduled",
                "run.pod_pending",
                "run.running",
                "run.succeeded"
            ]
        );
    }

    #[tokio::test]
    async fn reconcile_leaves_stable_runs_alone() {
        let repo = InMemoryRunRepository::new();
        let sessions = InMemorySessionRepository::default();
        let events = InMemoryEventRepository::default();
        let transitions = InMemoryTransitionRepository::new(repo.clone(), events.clone());
        let run = start_run(
            req(),
            &StubRuntime::new(None),
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &StubBroker,
        )
        .await
        .unwrap();

        // still pending (not ready yet) -> no change
        let n = reconcile_once(
            &StubRuntime::new(Some(RuntimeStatus::Pending)),
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &StubBroker,
        )
        .await
        .unwrap();
        assert_eq!(n, 0);
        assert_eq!(
            repo.get(run.id).await.unwrap().unwrap().state,
            RunState::PodPending
        );
    }

    #[tokio::test]
    async fn reconcile_respawns_missing_sandbox() {
        let repo = InMemoryRunRepository::new();
        let sessions = InMemorySessionRepository::default();
        let events = InMemoryEventRepository::default();
        let transitions = InMemoryTransitionRepository::new(repo.clone(), events.clone());
        let run = start_run(
            req(),
            &StubRuntime::new(None),
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &StubBroker,
        )
        .await
        .unwrap();
        // Advance PodPending -> Running so the run is live.
        reconcile_once(
            &StubRuntime::new(Some(RuntimeStatus::Ready)),
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &StubBroker,
        )
        .await
        .unwrap();

        // The sandbox vanishes (crash). The reconciler must re-spawn, not mark the
        // run terminal.
        let rt = StubRuntime::new(None);
        let n = reconcile_once(&rt, &repo, &sessions, &transitions, &StubBus, &StubBroker)
            .await
            .unwrap();
        assert_eq!(n, 1);
        assert_eq!(rt.spawn_count(), 1);
        // Still Running — `finished` is the only terminal signal.
        assert_eq!(
            repo.get(run.id).await.unwrap().unwrap().state,
            RunState::Running
        );
    }

    #[tokio::test]
    async fn reconcile_shards_only_touches_owned_runs() {
        let repo = InMemoryRunRepository::new();
        let sessions = InMemorySessionRepository::default();
        let events = InMemoryEventRepository::default();
        let transitions = InMemoryTransitionRepository::new(repo.clone(), events.clone());

        // Several runs (ids assigned sequentially from 1), more shards than runs so
        // some shards are guaranteed unowned.
        let mut runs = Vec::new();
        for _ in 0..4 {
            runs.push(
                start_run(
                    req(),
                    &StubRuntime::new(None),
                    &repo,
                    &sessions,
                    &transitions,
                    &StubBus,
                    &StubBroker,
                )
                .await
                .unwrap(),
            );
        }

        let shard_count = 16u32;
        let shard_of = |id| cat_herder_domain::shard::shard_for(id, shard_count);

        // Reconcile every shard in one call; each run must advance exactly once.
        let all_shards: HashSet<u32> = (0..shard_count).collect();
        let n = reconcile_shards(
            &all_shards,
            shard_count,
            &StubRuntime::new(Some(RuntimeStatus::Ready)),
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &StubBroker,
        )
        .await
        .unwrap();
        assert_eq!(n, runs.len());
        for run in &runs {
            assert_eq!(
                repo.get(run.id).await.unwrap().unwrap().state,
                RunState::Running
            );
        }

        // A shard set that owns none of the runs advances nothing (guaranteed to
        // exist since runs < shards).
        let owned: HashSet<u32> = runs.iter().map(|r| shard_of(r.id)).collect();
        let unowned: HashSet<u32> = (0..shard_count).filter(|s| !owned.contains(s)).collect();
        let n = reconcile_shards(
            &unowned,
            shard_count,
            &StubRuntime::new(Some(RuntimeStatus::Ready)),
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &StubBroker,
        )
        .await
        .unwrap();
        assert_eq!(n, 0);
    }

    #[tokio::test]
    async fn complete_run_marks_succeeded_and_syncs_display() {
        let repo = InMemoryRunRepository::new();
        let sessions = InMemorySessionRepository::default();
        let events = InMemoryEventRepository::default();
        let transitions = InMemoryTransitionRepository::new(repo.clone(), events.clone());
        let run = start_run(
            req(),
            &StubRuntime::new(None),
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &StubBroker,
        )
        .await
        .unwrap();
        let session_id = run.session_id.clone().unwrap();

        reconcile_once(
            &StubRuntime::new(Some(RuntimeStatus::Ready)),
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &StubBroker,
        )
        .await
        .unwrap();
        assert_eq!(
            repo.get(run.id).await.unwrap().unwrap().state,
            RunState::Running
        );

        let done = complete_run(
            run.id,
            Outcome::Succeeded,
            Completion {
                report: Some("# Done\n\nAll good.".into()),
                usage: None,
            },
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &StubBroker,
        )
        .await
        .unwrap();
        assert_eq!(done.state, RunState::Succeeded);
        assert_eq!(done.report.as_deref(), Some("# Done\n\nAll good."));
        assert!(done.finished_at.is_some());
        assert_eq!(
            sessions.get(&session_id).await.unwrap().unwrap().runs[0].state,
            RunState::Succeeded
        );
        // The report is persisted on the run.
        assert_eq!(
            repo.get(run.id).await.unwrap().unwrap().report.as_deref(),
            Some("# Done\n\nAll good.")
        );
    }

    #[tokio::test]
    async fn complete_run_marks_failed() {
        let repo = InMemoryRunRepository::new();
        let sessions = InMemorySessionRepository::default();
        let events = InMemoryEventRepository::default();
        let transitions = InMemoryTransitionRepository::new(repo.clone(), events.clone());
        let run = start_run(
            req(),
            &StubRuntime::new(None),
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &StubBroker,
        )
        .await
        .unwrap();
        reconcile_once(
            &StubRuntime::new(Some(RuntimeStatus::Ready)),
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &StubBroker,
        )
        .await
        .unwrap();

        let done = complete_run(
            run.id,
            Outcome::Failed,
            Completion::default(),
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &StubBroker,
        )
        .await
        .unwrap();
        assert_eq!(done.state, RunState::Failed);
    }

    #[tokio::test]
    async fn complete_run_is_idempotent() {
        let repo = InMemoryRunRepository::new();
        let sessions = InMemorySessionRepository::default();
        let events = InMemoryEventRepository::default();
        let transitions = InMemoryTransitionRepository::new(repo.clone(), events.clone());
        let run = start_run(
            req(),
            &StubRuntime::new(None),
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &StubBroker,
        )
        .await
        .unwrap();
        reconcile_once(
            &StubRuntime::new(Some(RuntimeStatus::Ready)),
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &StubBroker,
        )
        .await
        .unwrap();

        assert_eq!(
            complete_run(
                run.id,
                Outcome::Succeeded,
                Completion::default(),
                &repo,
                &sessions,
                &transitions,
                &StubBus,
                &StubBroker
            )
            .await
            .unwrap()
            .state,
            RunState::Succeeded
        );
        assert_eq!(
            complete_run(
                run.id,
                Outcome::Succeeded,
                Completion::default(),
                &repo,
                &sessions,
                &transitions,
                &StubBus,
                &StubBroker
            )
            .await
            .unwrap()
            .state,
            RunState::Succeeded
        );
    }

    #[tokio::test]
    async fn complete_run_unknown_run_errors() {
        let repo = InMemoryRunRepository::new();
        let sessions = InMemorySessionRepository::default();
        let events = InMemoryEventRepository::default();
        let transitions = InMemoryTransitionRepository::new(repo.clone(), events.clone());
        assert!(
            complete_run(
                RunId(999),
                Outcome::Succeeded,
                Completion::default(),
                &repo,
                &sessions,
                &transitions,
                &StubBus,
                &StubBroker
            )
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn interrupt_run_persists_approval_and_awaits_input() {
        let repo = InMemoryRunRepository::new();
        let sessions = InMemorySessionRepository::default();
        let events = InMemoryEventRepository::default();
        let transitions = InMemoryTransitionRepository::new(repo.clone(), events.clone());
        let approvals = InMemoryApprovalRepository::default();
        let run = start_run(
            req(),
            &StubRuntime::new(None),
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &StubBroker,
        )
        .await
        .unwrap();
        // Move PodPending -> Running so the interrupt can land.
        reconcile_once(
            &StubRuntime::new(Some(RuntimeStatus::Ready)),
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &StubBroker,
        )
        .await
        .unwrap();

        let approval = interrupt_run(
            run.id,
            "ckpt-123".into(),
            serde_json::json!({ "tool": "push", "args": { "branch": "main" } }),
            &approvals,
            &events,
            &repo,
            &sessions,
            &transitions,
            &StubBus,
        )
        .await
        .unwrap();

        assert_eq!(approval.status, ApprovalStatus::Pending);
        assert_eq!(approval.resume_token, "ckpt-123");
        assert_eq!(
            repo.get(run.id).await.unwrap().unwrap().state,
            RunState::AwaitingInput
        );
        // The log records approval.requested before run.awaiting_input.
        let types: Vec<String> = events
            .list(run.id, 0, 100)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.event_type)
            .collect();
        assert!(types.contains(&"approval.requested".to_string()));
        assert!(types.contains(&"run.awaiting_input".to_string()));
    }

    #[tokio::test]
    async fn respond_approval_approves_and_resumes() {
        let repo = InMemoryRunRepository::new();
        let sessions = InMemorySessionRepository::default();
        let events = InMemoryEventRepository::default();
        let transitions = InMemoryTransitionRepository::new(repo.clone(), events.clone());
        let approvals = InMemoryApprovalRepository::default();
        let run = start_run(
            req(),
            &StubRuntime::new(None),
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &StubBroker,
        )
        .await
        .unwrap();
        reconcile_once(
            &StubRuntime::new(Some(RuntimeStatus::Ready)),
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &StubBroker,
        )
        .await
        .unwrap();
        let approval = interrupt_run(
            run.id,
            "ckpt-123".into(),
            serde_json::json!({ "tool": "push" }),
            &approvals,
            &events,
            &repo,
            &sessions,
            &transitions,
            &StubBus,
        )
        .await
        .unwrap();

        let decided = respond_approval(
            &approval.id,
            true,
            Some("ok, go ahead".into()),
            &approvals,
            &events,
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &StubBroker,
        )
        .await
        .unwrap();

        assert_eq!(decided.status, ApprovalStatus::Approved);
        assert_eq!(decided.response.as_deref(), Some("ok, go ahead"));
        assert_eq!(
            repo.get(run.id).await.unwrap().unwrap().state,
            RunState::Running
        );
    }

    #[tokio::test]
    async fn respond_approval_rejects_and_cancels() {
        let repo = InMemoryRunRepository::new();
        let sessions = InMemorySessionRepository::default();
        let events = InMemoryEventRepository::default();
        let transitions = InMemoryTransitionRepository::new(repo.clone(), events.clone());
        let approvals = InMemoryApprovalRepository::default();
        let run = start_run(
            req(),
            &StubRuntime::new(None),
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &StubBroker,
        )
        .await
        .unwrap();
        reconcile_once(
            &StubRuntime::new(Some(RuntimeStatus::Ready)),
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &StubBroker,
        )
        .await
        .unwrap();
        let approval = interrupt_run(
            run.id,
            "ckpt-123".into(),
            serde_json::json!({ "tool": "push" }),
            &approvals,
            &events,
            &repo,
            &sessions,
            &transitions,
            &StubBus,
        )
        .await
        .unwrap();

        let decided = respond_approval(
            &approval.id,
            false,
            None,
            &approvals,
            &events,
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &StubBroker,
        )
        .await
        .unwrap();

        assert_eq!(decided.status, ApprovalStatus::Rejected);
        assert_eq!(
            repo.get(run.id).await.unwrap().unwrap().state,
            RunState::Cancelled
        );
    }

    #[tokio::test]
    async fn respond_approval_unknown_id_errors() {
        let repo = InMemoryRunRepository::new();
        let sessions = InMemorySessionRepository::default();
        let events = InMemoryEventRepository::default();
        let transitions = InMemoryTransitionRepository::new(repo.clone(), events.clone());
        let approvals = InMemoryApprovalRepository::default();
        assert!(
            respond_approval(
                "nope",
                true,
                None,
                &approvals,
                &events,
                &repo,
                &sessions,
                &transitions,
                &StubBus,
                &StubBroker
            )
            .await
            .is_err()
        );
    }

    /// Drive a run to `Succeeded` so the review use cases can operate on it.
    async fn run_to_succeeded() -> (
        RunId,
        InMemoryRunRepository,
        InMemorySessionRepository,
        InMemoryEventRepository,
        InMemoryTransitionRepository,
    ) {
        let repo = InMemoryRunRepository::new();
        let sessions = InMemorySessionRepository::default();
        let events = InMemoryEventRepository::default();
        let transitions = InMemoryTransitionRepository::new(repo.clone(), events.clone());
        let run = start_run(
            req(),
            &StubRuntime::new(None),
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &StubBroker,
        )
        .await
        .unwrap();
        reconcile_once(
            &StubRuntime::new(Some(RuntimeStatus::Ready)),
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &StubBroker,
        )
        .await
        .unwrap();
        complete_run(
            run.id,
            Outcome::Succeeded,
            Completion {
                report: Some("# Done".into()),
                usage: None,
            },
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &StubBroker,
        )
        .await
        .unwrap();
        (run.id, repo, sessions, events, transitions)
    }

    #[test]
    fn review_feedback_task_spec_embeds_verdict_and_feedback() {
        let spec = review_feedback_task_spec(
            "Fix the bug",
            RunId(42),
            ReviewVerdict::ChangesRequested,
            Some("octocat"),
            Some("add a test"),
        );
        assert!(spec.contains("changes requested"));
        assert!(spec.contains("octocat"));
        assert!(spec.contains("add a test"));
        assert!(spec.contains("Fix the bug"));
        assert!(spec.contains("run 42"));
    }

    #[tokio::test]
    async fn request_review_enters_awaiting_review_and_persists() {
        let (run_id, repo, sessions, events, transitions) = run_to_succeeded().await;
        let reviews = InMemoryReviewRepository::default();
        assert_eq!(
            repo.get(run_id).await.unwrap().unwrap().state,
            RunState::Succeeded
        );

        let deadline = SystemTime::now() + std::time::Duration::from_secs(3600);
        let review = request_review(
            run_id,
            deadline,
            &reviews,
            &events,
            &repo,
            &sessions,
            &transitions,
            &StubBus,
        )
        .await
        .unwrap()
        .expect("review requested");

        assert_eq!(review.status, ReviewStatus::Pending);
        assert_eq!(review.run_id, run_id);
        assert_eq!(
            repo.get(run_id).await.unwrap().unwrap().state,
            RunState::AwaitingReview
        );
        assert_eq!(reviews.list_pending().await.unwrap().len(), 1);
        let types: Vec<String> = events
            .list(run_id, 0, 100)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.event_type)
            .collect();
        assert!(types.contains(&"review.requested".to_string()));
        assert!(types.contains(&"run.awaiting_review".to_string()));
    }

    #[tokio::test]
    async fn decide_review_approves_to_reviewed() {
        let (run_id, repo, sessions, events, transitions) = run_to_succeeded().await;
        let reviews = InMemoryReviewRepository::default();
        let review = request_review(
            run_id,
            SystemTime::now(),
            &reviews,
            &events,
            &repo,
            &sessions,
            &transitions,
            &StubBus,
        )
        .await
        .unwrap()
        .expect("review requested");

        let decided = decide_review(
            &review.id,
            ReviewVerdict::Approved,
            Some("octocat".into()),
            None,
            &reviews,
            &events,
            &repo,
            &sessions,
            &transitions,
            &StubBus,
        )
        .await
        .unwrap();

        assert_eq!(decided.status, ReviewStatus::Decided);
        assert_eq!(decided.verdict, Some(ReviewVerdict::Approved));
        assert_eq!(decided.reviewer.as_deref(), Some("octocat"));
        assert_eq!(
            repo.get(run_id).await.unwrap().unwrap().state,
            RunState::Reviewed
        );
        assert!(reviews.list_pending().await.unwrap().is_empty());
        let types: Vec<String> = events
            .list(run_id, 0, 100)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.event_type)
            .collect();
        assert!(types.contains(&"review.decided".to_string()));
        assert!(types.contains(&"run.reviewed".to_string()));
    }

    #[tokio::test]
    async fn decide_review_changes_requested_rejects() {
        let (run_id, repo, sessions, events, transitions) = run_to_succeeded().await;
        let reviews = InMemoryReviewRepository::default();
        let review = request_review(
            run_id,
            SystemTime::now(),
            &reviews,
            &events,
            &repo,
            &sessions,
            &transitions,
            &StubBus,
        )
        .await
        .unwrap()
        .expect("review requested");

        let decided = decide_review(
            &review.id,
            ReviewVerdict::ChangesRequested,
            None,
            Some("please fix".into()),
            &reviews,
            &events,
            &repo,
            &sessions,
            &transitions,
            &StubBus,
        )
        .await
        .unwrap();

        assert_eq!(decided.verdict, Some(ReviewVerdict::ChangesRequested));
        assert_eq!(
            repo.get(run_id).await.unwrap().unwrap().state,
            RunState::Rejected
        );
        let types: Vec<String> = events
            .list(run_id, 0, 100)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.event_type)
            .collect();
        assert!(types.contains(&"run.rejected".to_string()));
    }

    #[tokio::test]
    async fn decide_review_unknown_id_errors() {
        let (run_id, repo, sessions, events, transitions) = run_to_succeeded().await;
        let reviews = InMemoryReviewRepository::default();
        assert!(
            decide_review(
                "nope",
                ReviewVerdict::Approved,
                None,
                None,
                &reviews,
                &events,
                &repo,
                &sessions,
                &transitions,
                &StubBus,
            )
            .await
            .is_err()
        );
        // The run is untouched.
        assert_eq!(
            repo.get(run_id).await.unwrap().unwrap().state,
            RunState::Succeeded
        );
    }

    #[tokio::test]
    async fn decide_review_already_decided_errors() {
        let (run_id, repo, sessions, events, transitions) = run_to_succeeded().await;
        let reviews = InMemoryReviewRepository::default();
        let review = request_review(
            run_id,
            SystemTime::now(),
            &reviews,
            &events,
            &repo,
            &sessions,
            &transitions,
            &StubBus,
        )
        .await
        .unwrap()
        .expect("review requested");
        decide_review(
            &review.id,
            ReviewVerdict::Approved,
            None,
            None,
            &reviews,
            &events,
            &repo,
            &sessions,
            &transitions,
            &StubBus,
        )
        .await
        .unwrap();

        assert!(
            decide_review(
                &review.id,
                ReviewVerdict::Rejected,
                None,
                None,
                &reviews,
                &events,
                &repo,
                &sessions,
                &transitions,
                &StubBus,
            )
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn request_review_is_idempotent_for_already_awaiting_review() {
        let (run_id, repo, sessions, events, transitions) = run_to_succeeded().await;
        let reviews = InMemoryReviewRepository::default();

        let first = request_review(
            run_id,
            SystemTime::now(),
            &reviews,
            &events,
            &repo,
            &sessions,
            &transitions,
            &StubBus,
        )
        .await
        .unwrap();
        assert!(first.is_some());
        assert_eq!(
            repo.get(run_id).await.unwrap().unwrap().state,
            RunState::AwaitingReview
        );

        // A re-delivered `finished` event must not create a second review.
        let second = request_review(
            run_id,
            SystemTime::now(),
            &reviews,
            &events,
            &repo,
            &sessions,
            &transitions,
            &StubBus,
        )
        .await
        .unwrap();
        assert!(second.is_none());
        assert_eq!(reviews.list_pending().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn start_run_mints_clone_credential_and_injects_secret() {
        let repo = InMemoryRunRepository::new();
        let sessions = InMemorySessionRepository::default();
        let events = InMemoryEventRepository::default();
        let transitions = InMemoryTransitionRepository::new(repo.clone(), events.clone());
        let broker = RecordingBroker::default();
        let runtime = StubRuntime::new(None);

        let run = start_run(
            req_with_repo("https://github.com/org/repo.git"),
            &runtime,
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &broker,
        )
        .await
        .unwrap();

        // The clone credential was minted for the canonical repo identity, and the
        // minted secret was handed to the runtime for out-of-band injection.
        assert_eq!(
            broker.minted(),
            vec![(run.id, "github.com/org/repo".to_string())]
        );
        assert_eq!(runtime.last_token().as_deref(), Some("ghs_fake_token"));
    }

    #[tokio::test]
    async fn terminal_revokes_clone_credential() {
        let repo = InMemoryRunRepository::new();
        let sessions = InMemorySessionRepository::default();
        let events = InMemoryEventRepository::default();
        let transitions = InMemoryTransitionRepository::new(repo.clone(), events.clone());
        let broker = RecordingBroker::default();
        let runtime = StubRuntime::new(None);

        let run = start_run(
            req_with_repo("https://github.com/org/repo.git"),
            &runtime,
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &broker,
        )
        .await
        .unwrap();
        assert_eq!(broker.revoked_runs(), vec![]);

        // Advance to Running, then complete — the terminal transition must revoke the
        // run's clone credential.
        reconcile_once(
            &StubRuntime::new(Some(RuntimeStatus::Ready)),
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &broker,
        )
        .await
        .unwrap();
        complete_run(
            run.id,
            Outcome::Succeeded,
            Completion::default(),
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &broker,
        )
        .await
        .unwrap();

        assert_eq!(broker.revoked_runs(), vec![run.id]);
    }

    #[tokio::test]
    async fn budget_enforced_and_pauses_run_on_exceed() {
        let repo = InMemoryRunRepository::new();
        let sessions = InMemorySessionRepository::default();
        let events = InMemoryEventRepository::default();
        let transitions = InMemoryTransitionRepository::new(repo.clone(), events.clone());
        let budgets = InMemoryBudgetRepository::default();
        let counter = InMemoryBudgetCounter::default();

        let run = start_run(
            req(),
            &StubRuntime::new(None),
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &StubBroker,
        )
        .await
        .unwrap();
        // Move PodPending -> Running so a budget pause can apply.
        reconcile_once(
            &StubRuntime::new(Some(RuntimeStatus::Ready)),
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &StubBroker,
        )
        .await
        .unwrap();

        issue_budget(&budgets, &counter, run.id, Some(100))
            .await
            .unwrap();

        // Two consumes within cap.
        let o1 = consume_budget(
            run.id,
            40,
            &counter,
            &budgets,
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &events,
        )
        .await
        .unwrap();
        assert_eq!(o1, ConsumeOutcome::Ok { consumed: 40 });
        let o2 = consume_budget(
            run.id,
            60,
            &counter,
            &budgets,
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &events,
        )
        .await
        .unwrap();
        assert_eq!(o2, ConsumeOutcome::Ok { consumed: 100 });

        // Over cap -> rejected + run paused + budget.exceeded recorded.
        let o3 = consume_budget(
            run.id,
            1,
            &counter,
            &budgets,
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &events,
        )
        .await
        .unwrap();
        assert_eq!(
            o3,
            ConsumeOutcome::Exceeded {
                cap: 100,
                consumed: 100
            }
        );
        assert_eq!(
            repo.get(run.id).await.unwrap().unwrap().state,
            RunState::BudgetPaused
        );
        let types: Vec<String> = events
            .list(run.id, 0, 100)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.event_type)
            .collect();
        assert!(types.contains(&"budget.exceeded".to_string()));
        assert!(types.contains(&"run.budget_paused".to_string()));
    }

    #[tokio::test]
    async fn raise_budget_resumes_paused_run() {
        let repo = InMemoryRunRepository::new();
        let sessions = InMemorySessionRepository::default();
        let events = InMemoryEventRepository::default();
        let transitions = InMemoryTransitionRepository::new(repo.clone(), events.clone());
        let budgets = InMemoryBudgetRepository::default();
        let counter = InMemoryBudgetCounter::default();

        let run = start_run(
            req(),
            &StubRuntime::new(None),
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &StubBroker,
        )
        .await
        .unwrap();
        reconcile_once(
            &StubRuntime::new(Some(RuntimeStatus::Ready)),
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &StubBroker,
        )
        .await
        .unwrap();

        issue_budget(&budgets, &counter, run.id, Some(10))
            .await
            .unwrap();
        consume_budget(
            run.id,
            20,
            &counter,
            &budgets,
            &repo,
            &sessions,
            &transitions,
            &StubBus,
            &events,
        )
        .await
        .unwrap();
        assert_eq!(
            repo.get(run.id).await.unwrap().unwrap().state,
            RunState::BudgetPaused
        );

        // Raise the cap -> resumed, and the counter can accept more spend.
        let resumed = raise_budget(
            run.id,
            1000,
            &budgets,
            &counter,
            &repo,
            &sessions,
            &transitions,
            &StubBus,
        )
        .await
        .unwrap();
        assert_eq!(resumed.state, RunState::Running);
        assert_eq!(budgets.get(run.id).await.unwrap().unwrap().cap, 1000);
        assert_eq!(
            consume_budget(
                run.id,
                500,
                &counter,
                &budgets,
                &repo,
                &sessions,
                &transitions,
                &StubBus,
                &events,
            )
            .await
            .unwrap(),
            ConsumeOutcome::Ok { consumed: 500 }
        );
    }

    #[tokio::test]
    async fn issue_budget_is_noop_without_a_cap() {
        let budgets = InMemoryBudgetRepository::default();
        let counter = InMemoryBudgetCounter::default();
        issue_budget(&budgets, &counter, RunId(7), None)
            .await
            .unwrap();
        assert!(budgets.get(RunId(7)).await.unwrap().is_none());
        assert_eq!(
            consume_budget(
                RunId(7),
                100,
                &counter,
                &budgets,
                &InMemoryRunRepository::new(),
                &InMemorySessionRepository::default(),
                &InMemoryTransitionRepository::new(
                    InMemoryRunRepository::new(),
                    InMemoryEventRepository::default()
                ),
                &StubBus,
                &InMemoryEventRepository::default(),
            )
            .await
            .unwrap(),
            ConsumeOutcome::Ok { consumed: 0 }
        );
    }
}
