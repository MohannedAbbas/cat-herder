//! Application services: orchestrate the domain ports into use cases.
//!
//! Generic over the port implementations (not `dyn`), per `PLAN.md` §5.

use std::time::SystemTime;

use cat_herder_domain::api::{SessionRun, is_safe_repo_url, new_session_id};
use cat_herder_domain::entities::{Run, RunId, RunState};
use cat_herder_domain::ports::{
    AgentRuntime, DomainResult, RunRepository, RunRequest, SessionRepository,
};
use cat_herder_domain::reconcile::reconcile_run;
use cat_herder_domain::state::is_terminal;

/// Use case: accept a trigger's request, persist it, spawn the agent, and advance the
/// lifecycle state machine through to `PodPending`. The reconciler (`reconcile_once`)
/// advances it to `Running` (and terminal states) once the sandbox is actually ready.
///
/// Each run is auto-linked to a freshly created session (title derived from the
/// task spec).
pub async fn start_run<R, P, S>(
    req: RunRequest,
    runtime: &R,
    repo: &P,
    sessions: &S,
) -> DomainResult<Run>
where
    R: AgentRuntime,
    P: RunRepository,
    S: SessionRepository,
{
    let run = Run {
        id: RunId(0), // assigned by the repository
        tenant: req.tenant,
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
    };

    let mut run = repo.save(&run).await?; // Queued (id assigned)

    // Auto-create a session and link the run to it.
    let session_id = new_session_id();
    sessions.create(&session_id, &session_title(&run.task_spec)).await?;
    run.session_id = Some(session_id.clone());
    run = repo.save(&run).await?;

    run.transition(RunState::Scheduled)?;
    run = repo.save(&run).await?;

    run.transition(RunState::PodPending)?;
    run = repo.save(&run).await?;

    // Project the run into the session's display rows (now `PodPending`).
    sessions.add_run(&session_id, &SessionRun::from(&run)).await?;

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
pub async fn reconcile_once<R, P, S>(runtime: &R, repo: &P, sessions: &S) -> DomainResult<usize>
where
    R: AgentRuntime,
    P: RunRepository,
    S: SessionRepository,
{
    let mut advanced = 0;

    for run in repo.list_all().await? {
        match run.state {
            RunState::PodPending | RunState::Running => {}
            _ => continue,
        }

        let status = runtime.status(run.id).await?;
        let transitions = reconcile_run(run.state, status);
        if transitions.is_empty() {
            continue;
        }

        let mut updated = run.clone();
        for next in transitions {
            updated.transition(next)?;
        }
        repo.save(&updated).await?;
        sessions.sync_run(&updated).await?;
        advanced += 1;

        // Reclaim the execution once the run is done (details are already persisted).
        if updated.state == RunState::Succeeded || is_terminal(updated.state) {
            if let Err(e) = runtime.terminate(updated.id).await {
                eprintln!("failed to terminate sandbox for run {}: {e}", updated.id.0);
            }
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

/// Use case: record the harness's completion of a run, advancing it to `Succeeded`
/// or `Failed` (from `Running`, or `PodPending` → `Running` → terminal). Idempotent
/// for runs that already reached a final state. Also syncs the session display row.
///
/// `report` is the agent's final markdown summary (from the harness `finished`
/// event); when present it is persisted onto the run (even for an already-final
/// run, so a late report is not lost).
pub async fn complete_run<P, S>(
    run_id: RunId,
    outcome: Outcome,
    report: Option<String>,
    repo: &P,
    sessions: &S,
) -> DomainResult<Run>
where
    P: RunRepository,
    S: SessionRepository,
{
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
    // persist a report that arrives now (e.g. a re-delivered `finished` event).
    if run.state == RunState::Succeeded || is_terminal(run.state) {
        let Some(report) = report else { return Ok(run) };
        if run.report.as_deref() == Some(report.as_str()) {
            return Ok(run);
        }
        let mut run = run;
        run.report = Some(report);
        let run = repo.save(&run).await?;
        return Ok(run);
    }

    let mut run = run;
    match run.state {
        RunState::Running => {
            run.transition(target)?;
        }
        RunState::PodPending => {
            run.transition(RunState::Running)?;
            run.transition(target)?;
        }
        // queued/scheduled/awaiting-input/etc. — not a runnable state; leave alone.
        _ => return Ok(run),
    }

    if report.is_some() {
        run.report = report;
    }

    let run = repo.save(&run).await?;
    sessions.sync_run(&run).await?;
    Ok(run)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cat_herder_domain::entities::{RunKind, TenantId};
    use cat_herder_domain::reconcile::RuntimeStatus;
    use cat_herder_store_inmemory::{InMemoryRunRepository, InMemorySessionRepository};

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

    fn req() -> RunRequest {
        RunRequest {
            tenant: TenantId(1),
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
        let run = start_run(req(), &StubRuntime::new(None), &repo, &sessions).await.unwrap();

        assert_eq!(run.state, RunState::PodPending);
        assert_ne!(run.id, RunId(0));

        let stored = repo.get(run.id).await.unwrap().unwrap();
        assert_eq!(stored.state, RunState::PodPending);
    }

    #[tokio::test]
    async fn start_run_creates_and_links_session() {
        let repo = InMemoryRunRepository::new();
        let sessions = InMemorySessionRepository::default();
        let run = start_run(req(), &StubRuntime::new(None), &repo, &sessions).await.unwrap();

        // The run is linked to a session, and that session has the run's display row
        // (projected at the final pre-reconcile state, PodPending).
        let session_id = run.session_id.as_deref().expect("run linked to a session");
        let session = sessions.get(session_id).await.unwrap().expect("session exists");
        assert_eq!(session.runs.len(), 1);
        assert_eq!(session.runs[0].id, format!("run-{}", run.id.0));
        assert_eq!(session.runs[0].task_spec, "test");
        assert_eq!(session.runs[0].state, RunState::PodPending);
    }

    #[tokio::test]
    async fn reconcile_walks_pod_pending_to_succeeded() {
        let repo = InMemoryRunRepository::new();
        let sessions = InMemorySessionRepository::default();
        let run = start_run(req(), &StubRuntime::new(None), &repo, &sessions).await.unwrap();
        assert_eq!(run.state, RunState::PodPending);
        let session_id = run.session_id.clone().unwrap();

        // ready -> running (display row synced)
        let n = reconcile_once(&StubRuntime::new(Some(RuntimeStatus::Ready)), &repo, &sessions)
            .await
            .unwrap();
        assert_eq!(n, 1);
        assert_eq!(repo.get(run.id).await.unwrap().unwrap().state, RunState::Running);
        assert_eq!(sessions.get(&session_id).await.unwrap().unwrap().runs[0].state, RunState::Running);

        // gone -> succeeded (display row synced)
        let n = reconcile_once(&StubRuntime::new(None), &repo, &sessions).await.unwrap();
        assert_eq!(n, 1);
        assert_eq!(repo.get(run.id).await.unwrap().unwrap().state, RunState::Succeeded);
        assert_eq!(sessions.get(&session_id).await.unwrap().unwrap().runs[0].state, RunState::Succeeded);
    }

    #[tokio::test]
    async fn reconcile_leaves_stable_runs_alone() {
        let repo = InMemoryRunRepository::new();
        let sessions = InMemorySessionRepository::default();
        let run = start_run(req(), &StubRuntime::new(None), &repo, &sessions).await.unwrap();

        // still pending (not ready yet) -> no change
        let n = reconcile_once(&StubRuntime::new(Some(RuntimeStatus::Pending)), &repo, &sessions)
            .await
            .unwrap();
        assert_eq!(n, 0);
        assert_eq!(repo.get(run.id).await.unwrap().unwrap().state, RunState::PodPending);
    }

    #[tokio::test]
    async fn complete_run_marks_succeeded_and_syncs_display() {
        let repo = InMemoryRunRepository::new();
        let sessions = InMemorySessionRepository::default();
        let run = start_run(req(), &StubRuntime::new(None), &repo, &sessions).await.unwrap();
        let session_id = run.session_id.clone().unwrap();

        reconcile_once(&StubRuntime::new(Some(RuntimeStatus::Ready)), &repo, &sessions)
            .await
            .unwrap();
        assert_eq!(repo.get(run.id).await.unwrap().unwrap().state, RunState::Running);

        let done = complete_run(
            run.id,
            Outcome::Succeeded,
            Some("# Done\n\nAll good.".into()),
            &repo,
            &sessions,
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
        let run = start_run(req(), &StubRuntime::new(None), &repo, &sessions).await.unwrap();
        reconcile_once(&StubRuntime::new(Some(RuntimeStatus::Ready)), &repo, &sessions)
            .await
            .unwrap();

        let done = complete_run(run.id, Outcome::Failed, None, &repo, &sessions).await.unwrap();
        assert_eq!(done.state, RunState::Failed);
    }

    #[tokio::test]
    async fn complete_run_is_idempotent() {
        let repo = InMemoryRunRepository::new();
        let sessions = InMemorySessionRepository::default();
        let run = start_run(req(), &StubRuntime::new(None), &repo, &sessions).await.unwrap();
        reconcile_once(&StubRuntime::new(Some(RuntimeStatus::Ready)), &repo, &sessions)
            .await
            .unwrap();

        assert_eq!(
            complete_run(run.id, Outcome::Succeeded, None, &repo, &sessions).await.unwrap().state,
            RunState::Succeeded
        );
        assert_eq!(
            complete_run(run.id, Outcome::Succeeded, None, &repo, &sessions).await.unwrap().state,
            RunState::Succeeded
        );
    }

    #[tokio::test]
    async fn complete_run_unknown_run_errors() {
        let repo = InMemoryRunRepository::new();
        let sessions = InMemorySessionRepository::default();
        assert!(complete_run(RunId(999), Outcome::Succeeded, None, &repo, &sessions).await.is_err());
    }
}
