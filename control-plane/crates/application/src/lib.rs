//! Application services: orchestrate the domain ports into use cases.
//!
//! Generic over the port implementations (not `dyn`), per `PLAN.md` §5.

use std::time::SystemTime;

use cat_herder_domain::api::{SessionRun, is_safe_repo_url, new_session_id};
use cat_herder_domain::entities::{Run, RunId, RunState, Usage};
use cat_herder_domain::events::Event;
use cat_herder_domain::ports::{
    AgentRuntime, DomainResult, EventBus, RunRepository, RunRequest, SessionRepository,
    TransitionRepository,
};
use cat_herder_domain::reconcile::reconcile_run;
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

/// Use case: accept a trigger's request, persist it, spawn the agent, and advance the
/// lifecycle state machine through to `PodPending`. The reconciler (`reconcile_once`)
/// advances it to `Running` (and terminal states) once the sandbox is actually ready.
///
/// Each run is auto-linked to a freshly created session (title derived from the
/// task spec).
pub async fn start_run<R, P, S, T, B>(
    req: RunRequest,
    runtime: &R,
    repo: &P,
    sessions: &S,
    transitions: &T,
    bus: &B,
) -> DomainResult<Run>
where
    R: AgentRuntime,
    P: RunRepository,
    S: SessionRepository,
    T: TransitionRepository,
    B: EventBus,
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
        repo: req.repo.filter(|r| is_safe_repo_url(r)),
        language: req.language,
        started_at: None,
        finished_at: None,
        pr_url: None,
        report: None,
        input_tokens: 0,
        output_tokens: 0,
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

    runtime.spawn(&run).await?;

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
/// state machine where possible. Returns the number of runs advanced.
///
/// When the reconciler itself reaches a final state (e.g. the stuck-run watchdog maps a
/// failed/liveness-killed or expired sandbox to `Stuck`/`TimedOut`), it also reclaims the
/// execution — mirroring the `finished` event path — so a dead sandbox doesn't linger.
pub async fn reconcile_once<R, P, S, T, B>(
    runtime: &R,
    repo: &P,
    sessions: &S,
    transitions: &T,
    bus: &B,
) -> DomainResult<usize>
where
    R: AgentRuntime,
    P: RunRepository,
    S: SessionRepository,
    T: TransitionRepository,
    B: EventBus,
{
    let mut advanced = 0;

    for run in repo.list_all().await? {
        match run.state {
            RunState::PodPending | RunState::Running => {}
            _ => continue,
        }

        let status = runtime.status(run.id).await?;
        let to_apply = reconcile_run(run.state, status);
        if to_apply.is_empty() {
            continue;
        }

        let mut updated = run.clone();
        for next in to_apply {
            // One atomic transition per state change, so the log records the full walk
            // (e.g. PodPending -> Running -> Succeeded) even on a fast-forward.
            persist_transition(&mut updated, next, transitions, bus).await?;
        }
        sessions.sync_run(&updated).await?;
        advanced += 1;

        // Reclaim the execution once the run is done (details are already persisted).
        let done = updated.state == RunState::Succeeded || is_terminal(updated.state);
        if done && let Err(e) = runtime.terminate(updated.id).await {
            eprintln!("failed to terminate sandbox for run {}: {e}", updated.id.0);
        }
    }

    Ok(advanced)
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
pub async fn complete_run<P, S, T, B>(
    run_id: RunId,
    outcome: Outcome,
    completion: Completion,
    repo: &P,
    sessions: &S,
    transitions: &T,
    bus: &B,
) -> DomainResult<Run>
where
    P: RunRepository,
    S: SessionRepository,
    T: TransitionRepository,
    B: EventBus,
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
        }
        RunState::PodPending => {
            persist_transition(&mut run, RunState::Running, transitions, bus).await?;
            persist_transition(&mut run, target, transitions, bus).await?;
        }
        // queued/scheduled/awaiting-input/etc. — not a runnable state; leave alone.
        _ => return Ok(run),
    }

    sessions.sync_run(&run).await?;
    Ok(run)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cat_herder_domain::entities::RunKind;
    use cat_herder_domain::ports::EventRepository;
    use cat_herder_domain::reconcile::RuntimeStatus;
    use cat_herder_store_inmemory::{
        InMemoryEventRepository, InMemoryRunRepository, InMemorySessionRepository,
        InMemoryTransitionRepository,
    };

    struct StubRuntime {
        status: Option<RuntimeStatus>,
    }

    impl StubRuntime {
        fn new(status: Option<RuntimeStatus>) -> Self {
            Self { status }
        }
    }

    impl AgentRuntime for StubRuntime {
        async fn spawn(&self, _run: &Run) -> DomainResult<()> {
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

    fn req() -> RunRequest {
        RunRequest {
            kind: RunKind::Agent,
            task_spec: "test".into(),
            dedup_key: String::new(),
            max_concurrency: 1,
            trigger: "api".into(),
            actor: None,
            model: None,
            repo: None,
            language: None,
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
        )
        .await
        .unwrap();

        assert_eq!(run.state, RunState::PodPending);
        assert_ne!(run.id, RunId(0));

        let stored = repo.get(run.id).await.unwrap().unwrap();
        assert_eq!(stored.state, RunState::PodPending);
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

        // gone -> succeeded (display row synced)
        let n = reconcile_once(
            &StubRuntime::new(None),
            &repo,
            &sessions,
            &transitions,
            &StubBus,
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
        )
        .await
        .unwrap();
        reconcile_once(
            &StubRuntime::new(Some(RuntimeStatus::Ready)),
            &repo,
            &sessions,
            &transitions,
            &StubBus,
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
        )
        .await
        .unwrap();
        reconcile_once(
            &StubRuntime::new(Some(RuntimeStatus::Ready)),
            &repo,
            &sessions,
            &transitions,
            &StubBus,
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
                &StubBus
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
                &StubBus
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
                &StubBus
            )
            .await
            .is_err()
        );
    }
}
