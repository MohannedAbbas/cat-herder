//! Ports (traits). The domain depends only on these, never on adapters.
//!
//! Native `async fn` in traits (stable since Rust 1.75) is used here; it is not
//! `dyn`-object-safe, so application services use generics (not `Box<dyn …>`),
//! per `PLAN.md` §5.

use crate::entities::{Run, RunId, RunState, TenantId};
use crate::reconcile::RuntimeStatus;

pub type DomainResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Normalized trigger input (see `PLAN.md` §5).
#[derive(Debug, Clone)]
pub struct RunRequest {
    pub tenant: TenantId,
    pub kind: crate::entities::RunKind,
    /// Rendered task spec — event-derived content is already marked untrusted (§11.6).
    pub task_spec: String,
    pub dedup_key: String,
    pub max_concurrency: u32,
}

/// Inbound (driving): every trigger source normalizes its events to `RunRequest`.
pub trait TriggerSource {
    async fn next(&mut self) -> Option<RunRequest>;
}

/// Outbound (driven): create/watch/signal/terminate agent executions.
pub trait AgentRuntime {
    async fn spawn(&self, run: &Run) -> DomainResult<()>;
    /// Report the execution's current status (`None` = gone).
    async fn status(&self, id: RunId) -> DomainResult<Option<RuntimeStatus>>;
    async fn signal(&self, id: RunId, state: RunState) -> DomainResult<()>;
    async fn terminate(&self, id: RunId) -> DomainResult<()>;
}

pub trait RunRepository {
    /// Persist (upsert) the run, assigning an id if it is `RunId(0)`, and return the
    /// stored run.
    async fn save(&self, run: &Run) -> DomainResult<Run>;
    async fn get(&self, id: RunId) -> DomainResult<Option<Run>>;
    async fn list_by_tenant(&self, tenant: TenantId) -> DomainResult<Vec<Run>>;
    /// All runs (used by the reconciler).
    async fn list_all(&self) -> DomainResult<Vec<Run>>;
}

pub trait EventBus {
    async fn publish(&self, run: RunId, event_type: &str, payload: &[u8]) -> DomainResult<()>;
}

pub trait ModelProvider {
    async fn complete(&self, run: RunId, prompt: &str) -> DomainResult<String>;
}

pub trait IdentityProvider {
    async fn validate(&self, token: &str) -> DomainResult<TenantId>;
}

pub trait CredentialBroker {
    async fn request(&self, run: RunId, target: &str, action: &str) -> DomainResult<String>;
}

pub trait Notifier {
    async fn notify(&self, tenant: TenantId, message: &str) -> DomainResult<()>;
}
