//! Application services: orchestrate the domain ports into use cases.
//!
//! Generic over the port implementations (not `dyn`), per `PLAN.md` §5.

use cat_herder_domain::entities::{Run, RunId, RunState};
use cat_herder_domain::ports::{AgentRuntime, DomainResult, EventBus, RunRepository, RunRequest};
use std::time::SystemTime;

/// Use case: accept a trigger's request, persist it, publish, and spawn the agent.
pub async fn start_run<R, P, B>(
    req: RunRequest,
    runtime: &R,
    repo: &P,
    bus: &B,
) -> DomainResult<Run>
where
    R: AgentRuntime,
    P: RunRepository,
    B: EventBus,
{
    let run = Run {
        id: RunId(0), // assigned by the repository on insert
        tenant: req.tenant,
        kind: req.kind,
        state: RunState::Queued,
        max_concurrency: req.max_concurrency,
        created_at: SystemTime::now(),
    };

    repo.save(&run).await?;
    bus.publish(run.id, "run.queued", b"{}").await?;
    runtime.spawn(&run).await?;

    Ok(run)
}
