//! Application services: orchestrate the domain ports into use cases.
//!
//! Generic over the port implementations (not `dyn`), per `PLAN.md` §5.

use std::time::SystemTime;

use cat_herder_domain::entities::{Run, RunId, RunState};
use cat_herder_domain::ports::{AgentRuntime, DomainResult, RunRepository, RunRequest};

/// Use case: accept a trigger's request, persist it, spawn the agent, and advance the
/// lifecycle state machine through to `Running`.
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

    run.transition(RunState::Running)?;
    run = repo.save(&run).await?;

    Ok(run)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cat_herder_domain::entities::{Run, RunId, RunKind, RunState, TenantId};
    use cat_herder_domain::ports::{AgentRuntime, DomainResult};
    use cat_herder_store_inmemory::InMemoryRunRepository;

    struct FakeRuntime;

    impl AgentRuntime for FakeRuntime {
        async fn spawn(&self, _run: &Run) -> DomainResult<()> {
            Ok(())
        }
        async fn signal(&self, _id: RunId, _state: RunState) -> DomainResult<()> {
            Ok(())
        }
        async fn terminate(&self, _id: RunId) -> DomainResult<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn start_run_advances_to_running_and_persists() {
        let repo = InMemoryRunRepository::new();
        let run = start_run(
            RunRequest {
                tenant: TenantId(1),
                kind: RunKind::Agent,
                task_spec: "test".into(),
                dedup_key: String::new(),
                max_concurrency: 1,
            },
            &FakeRuntime,
            &repo,
        )
        .await
        .unwrap();

        assert_eq!(run.state, RunState::Running);
        assert_ne!(run.id, RunId(0));

        let stored = repo.get(run.id).await.unwrap().unwrap();
        assert_eq!(stored.state, RunState::Running);
    }
}
