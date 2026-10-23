//! Application services: orchestrate the domain ports into use cases.
//!
//! Generic over the port implementations (not `dyn`), per `PLAN.md` §5.

use std::time::SystemTime;

use cat_herder_domain::entities::{Run, RunId, RunState};
use cat_herder_domain::ports::{AgentRuntime, DomainResult, RunRepository, RunRequest};
use cat_herder_domain::reconcile::reconcile_run;

/// Use case: accept a trigger's request, persist it, spawn the agent, and advance the
/// lifecycle state machine through to `PodPending`. The reconciler (`reconcile_once`)
/// advances it to `Running` (and terminal states) once the sandbox is actually ready.
pub async fn start_run<R, P>(req: RunRequest, runtime: &R, repo: &P) -> DomainResult<Run>
where
    R: AgentRuntime,
    P: RunRepository,
{
    let run = Run {
        id: RunId(0), // assigned by the repository
        tenant: req.tenant,
        kind: req.kind,
        state: RunState::Queued,
        max_concurrency: req.max_concurrency,
        created_at: SystemTime::now(),
    };

    let mut run = repo.save(&run).await?; // Queued

    run.transition(RunState::Scheduled)?;
    run = repo.save(&run).await?;

    run.transition(RunState::PodPending)?;
    run = repo.save(&run).await?;

    runtime.spawn(&run).await?;

    Ok(run) // PodPending — the reconciler advances from here
}

/// Reconcile all active runs against their execution status, advancing the lifecycle
/// state machine where possible. Returns the number of runs advanced.
pub async fn reconcile_once<R, P>(runtime: &R, repo: &P) -> DomainResult<usize>
where
    R: AgentRuntime,
    P: RunRepository,
{
    let mut advanced = 0;

    for run in repo.list_all().await? {
        match run.state {
            RunState::PodPending | RunState::Running => {
                let status = runtime.status(run.id).await?;
                if let Some(next) = reconcile_run(run.state, status) {
                    let mut updated = run.clone();
                    updated.transition(next)?;
                    repo.save(&updated).await?;
                    advanced += 1;
                }
            }
            _ => {}
        }
    }

    Ok(advanced)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cat_herder_domain::entities::{RunKind, TenantId};
    use cat_herder_domain::reconcile::RuntimeStatus;
    use cat_herder_store_inmemory::InMemoryRunRepository;

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
        }
    }

    #[tokio::test]
    async fn start_run_advances_to_pod_pending_and_persists() {
        let repo = InMemoryRunRepository::new();
        let run = start_run(req(), &StubRuntime::new(None), &repo).await.unwrap();

        assert_eq!(run.state, RunState::PodPending);
        assert_ne!(run.id, RunId(0));

        let stored = repo.get(run.id).await.unwrap().unwrap();
        assert_eq!(stored.state, RunState::PodPending);
    }

    #[tokio::test]
    async fn reconcile_walks_pod_pending_to_succeeded() {
        let repo = InMemoryRunRepository::new();
        let run = start_run(req(), &StubRuntime::new(None), &repo).await.unwrap();
        assert_eq!(run.state, RunState::PodPending);

        // ready -> running
        let n = reconcile_once(&StubRuntime::new(Some(RuntimeStatus::Ready)), &repo)
            .await
            .unwrap();
        assert_eq!(n, 1);
        assert_eq!(repo.get(run.id).await.unwrap().unwrap().state, RunState::Running);

        // gone -> succeeded
        let n = reconcile_once(&StubRuntime::new(None), &repo).await.unwrap();
        assert_eq!(n, 1);
        assert_eq!(repo.get(run.id).await.unwrap().unwrap().state, RunState::Succeeded);
    }

    #[tokio::test]
    async fn reconcile_leaves_stable_runs_alone() {
        let repo = InMemoryRunRepository::new();
        let run = start_run(req(), &StubRuntime::new(None), &repo).await.unwrap();

        // still pending (not ready yet) -> no change
        let n = reconcile_once(&StubRuntime::new(Some(RuntimeStatus::Pending)), &repo)
            .await
            .unwrap();
        assert_eq!(n, 0);
        assert_eq!(repo.get(run.id).await.unwrap().unwrap().state, RunState::PodPending);
    }
}
