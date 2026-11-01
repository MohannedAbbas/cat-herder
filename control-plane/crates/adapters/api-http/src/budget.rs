//! Budget engine wiring (`PLAN.md` §12): the durable cap selector, the atomic counter
//! selector, and the gRPC `BudgetControl` service the metering proxy calls before each
//! model call. The control plane owns the atomic counter (Redis in production,
//! in-memory for single-replica dev) and transitions a run to `BudgetPaused` on over-cap.

use cat_herder_application::{consume_budget, raise_budget};
use cat_herder_domain::entities::{Budget, RunId};
use cat_herder_domain::ports::{BudgetCounter, BudgetRepository, ConsumeOutcome, DomainResult};
use cat_herder_store_inmemory::{InMemoryBudgetCounter, InMemoryBudgetRepository};
use cat_herder_store_postgres::PostgresBudgetRepository;
use cat_herder_store_redis::RedisBudgetCounter;
use tonic::{Request, Response, Status};

use crate::AppState;
use crate::grpc::control::v1::budget_control_server::{BudgetControl, BudgetControlServer};
use crate::grpc::control::v1::{
    ConsumeRequest, ConsumeResponse, RemainingRequest, RemainingResponse, ResetRequest,
    ResetResponse,
};

/// Durable budget-cap store selector: Postgres in production, in-memory otherwise.
#[derive(Clone)]
pub enum BudgetRepo {
    InMemory(InMemoryBudgetRepository),
    Postgres(PostgresBudgetRepository),
}

impl BudgetRepository for BudgetRepo {
    async fn save(&self, budget: &Budget) -> DomainResult<Budget> {
        match self {
            BudgetRepo::InMemory(r) => r.save(budget).await,
            BudgetRepo::Postgres(p) => p.save(budget).await,
        }
    }
    async fn get(&self, run_id: RunId) -> DomainResult<Option<Budget>> {
        match self {
            BudgetRepo::InMemory(r) => r.get(run_id).await,
            BudgetRepo::Postgres(p) => p.get(run_id).await,
        }
    }

    async fn delete(&self, run_id: RunId) -> DomainResult<()> {
        match self {
            BudgetRepo::InMemory(r) => r.delete(run_id).await,
            BudgetRepo::Postgres(p) => p.delete(run_id).await,
        }
    }
}

/// Atomic budget-counter selector: Redis in production, in-memory otherwise.
#[derive(Clone)]
pub enum BudgetCounterImpl {
    InMemory(InMemoryBudgetCounter),
    Redis(Box<RedisBudgetCounter>),
}

impl BudgetCounter for BudgetCounterImpl {
    async fn consume(&self, run_id: RunId, cap: u64, tokens: u64) -> DomainResult<ConsumeOutcome> {
        match self {
            BudgetCounterImpl::InMemory(c) => c.consume(run_id, cap, tokens).await,
            BudgetCounterImpl::Redis(c) => c.consume(run_id, cap, tokens).await,
        }
    }
    async fn remaining(&self, run_id: RunId, cap: u64) -> DomainResult<u64> {
        match self {
            BudgetCounterImpl::InMemory(c) => c.remaining(run_id, cap).await,
            BudgetCounterImpl::Redis(c) => c.remaining(run_id, cap).await,
        }
    }
    async fn reset(&self, run_id: RunId) -> DomainResult<()> {
        match self {
            BudgetCounterImpl::InMemory(c) => c.reset(run_id).await,
            BudgetCounterImpl::Redis(c) => c.reset(run_id).await,
        }
    }
}

/// Build the tonic service for the budget control channel.
pub fn budget_server(state: AppState) -> BudgetControlServer<BudgetControlService> {
    BudgetControlServer::new(BudgetControlService { state })
}

/// The `BudgetControl` service: the metering proxy's enforcement point. `Consume`
/// atomically meters a call and pauses the run on over-cap; `Remaining`/`Reset` serve
/// the harness fast-path and the budget lease/raise respectively.
#[derive(Clone)]
pub struct BudgetControlService {
    state: AppState,
}

impl BudgetControlService {
    /// The run's durable cap, or `0` (unbounded).
    async fn cap(&self, run_id: RunId) -> u64 {
        self.state
            .budgets
            .get(run_id)
            .await
            .ok()
            .flatten()
            .map(|b| b.cap)
            .unwrap_or(0)
    }
}

#[tonic::async_trait]
impl BudgetControl for BudgetControlService {
    async fn consume(
        &self,
        request: Request<ConsumeRequest>,
    ) -> Result<Response<ConsumeResponse>, Status> {
        let req = request.into_inner();
        let run_id = RunId(req.run_id);

        let outcome = consume_budget(
            run_id,
            req.tokens,
            &self.state.budget_counter,
            &self.state.budgets,
            &self.state.repo,
            &self.state.sessions,
            &self.state.transitions,
            &self.state.bus,
            &self.state.events,
            &self.state.run_status_notifier,
        )
        .await
        .map_err(|e| Status::internal(format!("budget consume failed: {e}")))?;

        let (exceeded, consumed) = match outcome {
            ConsumeOutcome::Ok { consumed } => (false, consumed),
            ConsumeOutcome::Exceeded { consumed, .. } => (true, consumed),
        };
        let cap = self.cap(run_id).await;
        let remaining = if exceeded {
            0
        } else {
            cap.saturating_sub(consumed)
        };

        Ok(Response::new(ConsumeResponse {
            exceeded,
            consumed,
            remaining,
        }))
    }

    async fn remaining(
        &self,
        request: Request<RemainingRequest>,
    ) -> Result<Response<RemainingResponse>, Status> {
        let run_id = RunId(request.into_inner().run_id);
        let cap = self.cap(run_id).await;
        let remaining = if cap == 0 {
            u64::MAX // unbounded
        } else {
            self.state
                .budget_counter
                .remaining(run_id, cap)
                .await
                .map_err(|e| Status::internal(format!("budget remaining failed: {e}")))?
        };
        Ok(Response::new(RemainingResponse { remaining }))
    }

    async fn reset(
        &self,
        request: Request<ResetRequest>,
    ) -> Result<Response<ResetResponse>, Status> {
        let run_id = RunId(request.into_inner().run_id);
        self.state
            .budget_counter
            .reset(run_id)
            .await
            .map_err(|e| Status::internal(format!("budget reset failed: {e}")))?;
        Ok(Response::new(ResetResponse {}))
    }
}

/// Raise a run's budget cap and resume a `BudgetPaused` run (the HTTP handler helper).
pub async fn raise_budget_for_run(state: &AppState, run_id: RunId, cap: u64) -> Result<(), String> {
    raise_budget(
        run_id,
        cap,
        &state.budgets,
        &state.budget_counter,
        &state.repo,
        &state.sessions,
        &state.transitions,
        &state.bus,
        &state.run_status_notifier,
    )
    .await
    .map(|_| ())
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cat_herder_domain::entities::Budget;

    #[tokio::test]
    async fn budget_repo_and_counter_inmemory_dispatch() {
        let repo = BudgetRepo::InMemory(InMemoryBudgetRepository::default());
        let counter = BudgetCounterImpl::InMemory(InMemoryBudgetCounter::default());

        repo.save(&Budget {
            run_id: RunId(7),
            cap: 100,
            updated_at: std::time::SystemTime::now(),
        })
        .await
        .unwrap();
        assert_eq!(repo.get(RunId(7)).await.unwrap().unwrap().cap, 100);

        assert_eq!(
            counter.consume(RunId(7), 100, 40).await.unwrap(),
            ConsumeOutcome::Ok { consumed: 40 }
        );
        assert_eq!(counter.remaining(RunId(7), 100).await.unwrap(), 60);
        assert_eq!(
            counter.consume(RunId(7), 100, 61).await.unwrap(),
            ConsumeOutcome::Exceeded {
                cap: 100,
                consumed: 40
            }
        );
        counter.reset(RunId(7)).await.unwrap();
        assert_eq!(counter.remaining(RunId(7), 100).await.unwrap(), 100);
    }

    #[tokio::test]
    async fn cap_helper_defaults_to_zero_when_no_budget() {
        let service = BudgetControlService {
            state: AppState::default(),
        };
        assert_eq!(service.cap(RunId(9)).await, 0);
    }
}
