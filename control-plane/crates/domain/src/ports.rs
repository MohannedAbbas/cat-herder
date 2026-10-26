//! Ports (traits). The domain depends only on these, never on adapters.
//!
//! Native `async fn` in traits (stable since Rust 1.75) is used here; it is not
//! `dyn`-object-safe, so application services use generics (not `Box<dyn …>`),
//! per `PLAN.md` §5.

use crate::api::{AgentTemplate, AuditEntry, Model, Provider, Session, SessionRun, Tool};
use crate::entities::{GitRepo, Run, RunId, RunState};
use crate::events::Event;
use crate::reconcile::RuntimeStatus;

pub type DomainResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Normalized trigger input (see `PLAN.md` §5).
#[derive(Debug, Clone)]
pub struct RunRequest {
    pub kind: crate::entities::RunKind,
    /// Rendered task spec — event-derived content is already marked untrusted (§11.6).
    pub task_spec: String,
    pub dedup_key: String,
    pub max_concurrency: u32,
    /// Trigger channel ("api" | "webhook" | "mcp" | …).
    pub trigger: String,
    /// Optional identity of the requester (until SSO is wired).
    pub actor: Option<String>,
    pub model: Option<String>,
    pub repo: Option<String>,
    pub language: Option<String>,
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
    /// All runs.
    async fn list_all(&self) -> DomainResult<Vec<Run>>;
}

/// Read-only catalog of agent templates (reference data, `PLAN.md` §13).
pub trait AgentRepository {
    async fn list(&self) -> DomainResult<Vec<AgentTemplate>>;
    async fn get(&self, id: &str) -> DomainResult<Option<AgentTemplate>>;
    /// Upsert an agent template.
    async fn save(&self, agent: &AgentTemplate) -> DomainResult<AgentTemplate>;
    /// Delete an agent template.
    async fn delete(&self, id: &str) -> DomainResult<()>;
}

/// LLM provider registry.
pub trait ProviderRepository {
    async fn list(&self) -> DomainResult<Vec<Provider>>;
    async fn get(&self, id: &str) -> DomainResult<Option<Provider>>;
    async fn save(&self, provider: &Provider) -> DomainResult<Provider>;
    async fn delete(&self, id: &str) -> DomainResult<()>;
}

/// Model registry (models belong to a provider).
pub trait ModelRepository {
    async fn list(&self) -> DomainResult<Vec<Model>>;
    async fn get(&self, id: &str) -> DomainResult<Option<Model>>;
    async fn save(&self, model: &Model) -> DomainResult<Model>;
    async fn delete(&self, id: &str) -> DomainResult<()>;
}

/// Append-only audit log of admin actions.
pub trait AuditRepository {
    async fn record(
        &self,
        actor: &str,
        action: &str,
        resource: &str,
        resource_id: &str,
    ) -> DomainResult<()>;
    /// Most recent entries first.
    async fn list(&self, limit: i64) -> DomainResult<Vec<AuditEntry>>;
}

/// Configurable list of available agent tools.
pub trait ToolRepository {
    async fn list(&self) -> DomainResult<Vec<Tool>>;
    async fn get(&self, id: &str) -> DomainResult<Option<Tool>>;
    async fn save(&self, tool: &Tool) -> DomainResult<Tool>;
    async fn delete(&self, id: &str) -> DomainResult<()>;
}

/// Git repositories (with encrypted tokens) that agents can clone (see §10).
pub trait GitRepoRepository {
    async fn list(&self) -> DomainResult<Vec<GitRepo>>;
    async fn get(&self, id: &str) -> DomainResult<Option<GitRepo>>;
    async fn save(&self, repo: &GitRepo) -> DomainResult<GitRepo>;
    async fn delete(&self, id: &str) -> DomainResult<()>;
}

/// Key-value settings (e.g. the webhook secret).
pub trait SettingRepository {
    async fn get(&self, key: &str) -> DomainResult<Option<String>>;
    async fn set(&self, key: &str, value: &str) -> DomainResult<()>;
}

/// Read-only session store (runs grouped into sessions, `PLAN.md` §12).
pub trait SessionRepository {
    async fn list(&self) -> DomainResult<Vec<Session>>;
    async fn get(&self, id: &str) -> DomainResult<Option<Session>>;
    /// Create a session with the given id and title (empty runs).
    async fn create(&self, id: &str, title: &str) -> DomainResult<()>;
    /// Append a display run to an existing session.
    async fn add_run(&self, session_id: &str, run: &SessionRun) -> DomainResult<()>;
    /// Update a session's display run (keyed `run-{id}`) to match the lifecycle
    /// run's current state and timing (called by the reconciler as it advances).
    async fn sync_run(&self, run: &Run) -> DomainResult<()>;
}

/// Durable, append-only event log (`PLAN.md` §9). The control plane is the sole
/// writer; `append` assigns the per-run monotonic `seq`.
pub trait EventRepository {
    /// Persist `event` for `run_id`, assigning `seq`. Returns the stored event.
    async fn append(&self, run_id: RunId, event: &Event) -> DomainResult<Event>;
    /// Replay events for a run strictly after `after_seq`, ascending, capped at `limit`.
    async fn list(&self, run_id: RunId, after_seq: i64, limit: i64) -> DomainResult<Vec<Event>>;
}

/// Fan-out transport for events (`PLAN.md` §9) — e.g. NATS JetStream. Best-effort:
/// the durable authority is [`EventRepository`]; a failed publish must not fail the
/// append.
pub trait EventBus {
    async fn publish(&self, event: &Event) -> DomainResult<()>;
}

pub trait ModelProvider {
    async fn complete(&self, run: RunId, prompt: &str) -> DomainResult<String>;
}

pub trait IdentityProvider {
    async fn validate(&self, token: &str) -> DomainResult<()>;
}

pub trait CredentialBroker {
    async fn request(&self, run: RunId, target: &str, action: &str) -> DomainResult<String>;
}

pub trait Notifier {
    async fn notify(&self, message: &str) -> DomainResult<()>;
}
