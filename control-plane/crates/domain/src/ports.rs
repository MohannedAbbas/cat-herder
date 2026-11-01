//! Ports (traits). The domain depends only on these, never on adapters.
//!
//! Native `async fn` in traits (stable since Rust 1.75) is used here; it is not
//! `dyn`-object-safe, so application services use generics (not `Box<dyn …>`),
//! per `PLAN.md` §5.

use crate::api::{AgentTemplate, AuditEntry, Model, Provider, Session, SessionRun, Tool};
use crate::entities::{
    ApprovalRequest, ApprovalStatus, Budget, Checkpoint, CredentialGrant, GitRepo, Review,
    ReviewVerdict, Run, RunId, RunState, WorkflowDefinition,
};
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
    /// Harness image to run this execution on (per-agent). `None` = global default.
    pub image: Option<String>,
    pub repo: Option<String>,
    pub language: Option<String>,
    /// A configurable workflow to run (its id), used when `kind == Workflow` (§7).
    pub workflow_ref: Option<String>,
    /// The isolation tier the run executes under (§13).
    pub runtime_tier: crate::entities::RuntimeTier,
    /// Token budget cap for the run (`PLAN.md` §12). `None`/`0` = unbounded.
    pub budget: Option<u64>,
    /// Optional URL the control plane POSTs the terminal result to (reply channel).
    pub callback_url: Option<String>,
}

/// Inbound (driving): every trigger source normalizes its events to `RunRequest`.
pub trait TriggerSource {
    async fn next(&mut self) -> Option<RunRequest>;
}

/// Outbound (driven): create/watch/signal/terminate agent executions.
pub trait AgentRuntime {
    /// Spawn the execution for `run`. `git_token` is an already-minted credential the
    /// runtime injects into the pod (out-of-band, e.g. `GIT_TOKEN` → git askpass);
    /// `None` means no broker-minted credential is available (the runtime may fall back
    /// to a registered repo token, or none for public repos).
    async fn spawn(&self, run: &Run, git_token: Option<&str>) -> DomainResult<()>;
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
    /// Hard-delete a run (Postgres cascades its events/checkpoints/etc. via the FK).
    async fn delete(&self, id: RunId) -> DomainResult<()>;
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

/// Configurable workflow catalog (`PLAN.md` §7): ordered, user-editable agent
/// pipelines that a `workflow` run compiles into a LangGraph `StateGraph` at start.
pub trait WorkflowRepository {
    async fn list(&self) -> DomainResult<Vec<WorkflowDefinition>>;
    async fn get(&self, id: &str) -> DomainResult<Option<WorkflowDefinition>>;
    /// Upsert a workflow definition (validated by the caller: non-empty, unique steps).
    async fn save(&self, workflow: &WorkflowDefinition) -> DomainResult<WorkflowDefinition>;
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
    /// Remove a run's display row (keyed `run-{id}`) from every session
    /// (retention/deletion, §11.7).
    async fn delete_run(&self, run_id: RunId) -> DomainResult<()>;
}

/// Durable, append-only event log (`PLAN.md` §9). The control plane is the sole
/// writer; `append` assigns the per-run monotonic `seq`.
pub trait EventRepository {
    /// Persist `event` for `run_id`, assigning `seq`. Returns the stored event.
    async fn append(&self, run_id: RunId, event: &Event) -> DomainResult<Event>;
    /// Replay events for a run strictly after `after_seq`, ascending, capped at `limit`.
    async fn list(&self, run_id: RunId, after_seq: i64, limit: i64) -> DomainResult<Vec<Event>>;
    /// Hard-delete every event for a run (retention/deletion, §11.7).
    async fn delete_run(&self, run_id: RunId) -> DomainResult<()>;
}

/// Durable LangGraph checkpoint store (`PLAN.md` §6). The control plane is the sole
/// writer; `append` assigns the per-run monotonic `seq`. The payload is opaque — the
/// control plane persists the harness's serialized `CheckpointTuple` without parsing it.
pub trait CheckpointRepository {
    /// Persist `checkpoint` for `run_id`, assigning `seq`. Returns the stored checkpoint.
    async fn append(&self, run_id: RunId, checkpoint: &Checkpoint) -> DomainResult<Checkpoint>;
    /// The latest checkpoint for a run (highest `seq`), or `None`.
    async fn latest(&self, run_id: RunId) -> DomainResult<Option<Checkpoint>>;
    /// Hard-delete every checkpoint for a run (retention/deletion, §11.7).
    async fn delete_run(&self, run_id: RunId) -> DomainResult<()>;
}

/// Durable human-in-the-loop approvals (`PLAN.md` §10). A request is created when the
/// harness reports an interrupt; `decide` records the human's verdict and response.
pub trait ApprovalRepository {
    /// Persist a new (pending) approval request, assigning an id if empty.
    async fn create(&self, approval: &ApprovalRequest) -> DomainResult<ApprovalRequest>;
    async fn get(&self, id: &str) -> DomainResult<Option<ApprovalRequest>>;
    /// Record the human's decision on an approval request.
    async fn decide(
        &self,
        id: &str,
        status: ApprovalStatus,
        response: Option<String>,
    ) -> DomainResult<ApprovalRequest>;
    /// Every pending approval, oldest first (the approval queue).
    async fn list_pending(&self) -> DomainResult<Vec<ApprovalRequest>>;
    /// Hard-delete every approval for a run (retention/deletion, §11.7).
    async fn delete_run(&self, run_id: RunId) -> DomainResult<()>;
}

/// Durable result reviews (`PLAN.md` §10). A review is created when a run producing
/// reviewable changes (a diff/PR) enters `AwaitingReview`; `decide` records the
/// reviewer's verdict and the run leaves via `Reviewed`/`Rejected`.
pub trait ReviewRepository {
    /// Persist a new (pending) review request, assigning an id if empty.
    async fn create(&self, review: &Review) -> DomainResult<Review>;
    async fn get(&self, id: &str) -> DomainResult<Option<Review>>;
    /// The review queue: every pending review, oldest first.
    async fn list_pending(&self) -> DomainResult<Vec<Review>>;
    /// Record the reviewer's verdict (and optional feedback) on a review.
    async fn decide(
        &self,
        id: &str,
        verdict: ReviewVerdict,
        reviewer: Option<String>,
        feedback: Option<String>,
    ) -> DomainResult<Review>;
    /// Hard-delete every review for a run (retention/deletion, §11.7).
    async fn delete_run(&self, run_id: RunId) -> DomainResult<()>;
}

/// Fan-out transport for events (`PLAN.md` §9) — e.g. NATS JetStream. Best-effort:
/// the durable authority is [`EventRepository`]; a failed publish must not fail the
/// append.
pub trait EventBus {
    async fn publish(&self, event: &Event) -> DomainResult<()>;
}

/// Outbound (driven): notify an external client (e.g. an MCP session) that a run's
/// lifecycle state changed. Fire-and-forget — the durable authority is the transition
/// repository; a failed notify must not fail the transition it follows (mirrors
/// [`EventBus`]).
pub trait RunStatusNotifier {
    async fn notify(&self, run: &Run) -> DomainResult<()>;
}

/// Lease primitive for the sharded, lease-based reconciler (`PLAN.md` §6).
///
/// A lease is the mutual-exclusion token that grants a leader ownership of a shard
/// (a bucket of `run_id` hashes) for a bounded time. The TTL is an adapter concern
/// (configured at construction); callers renew before it expires. A lease that is
/// not renewed lapses, and another leader can then take the shard over — that is the
/// crash-recovery mechanism: a dead leader's shards are re-acquired and their
/// non-terminal runs re-attached.
pub trait LeaseManager {
    /// Attempt to acquire the `shard` lease as `owner`. Returns `Ok(true)` if this
    /// caller now holds it (freshly acquired, or taken over after the previous
    /// holder's lease lapsed); `Ok(false)` if another live owner holds it.
    async fn acquire(&self, shard: u32, owner: &str) -> DomainResult<bool>;
    /// Renew an owned lease, extending its expiry. Returns `Ok(false)` if the lease
    /// was lost (lapsed and taken over by another owner) and must be re-acquired.
    async fn renew(&self, shard: u32, owner: &str) -> DomainResult<bool>;
    /// Release an owned lease (best-effort, no-op if no longer owned).
    async fn release(&self, shard: u32, owner: &str) -> DomainResult<()>;
}

/// Persists a lifecycle transition atomically (`PLAN.md` §6/§9): the run's new state
/// and its lifecycle event land together in one transaction, so the event log never
/// diverges from the run's lifecycle. `save_transition` upserts the run (assigning its
/// id if new) and appends the event with that id and a fresh per-run `seq`.
pub trait TransitionRepository {
    async fn save_transition(&self, run: &Run, event: &Event) -> DomainResult<(Run, Event)>;
}

pub trait ModelProvider {
    async fn complete(&self, run: RunId, prompt: &str) -> DomainResult<String>;
}

pub trait IdentityProvider {
    async fn validate(&self, token: &str) -> DomainResult<()>;
}

/// A freshly minted credential and its grant record. The `secret` is returned to the
/// **trusted** caller only (the control plane / broker), which injects it out-of-band —
/// into a pod env, a credential helper, or a sidecar — and **never** into model context.
pub struct MintedCredential {
    pub grant: CredentialGrant,
    pub secret: String,
}

/// Just-in-time credential brokering (`PLAN.md` §11.3).
///
/// Mints scoped, short-lived, revocable credentials for a run's declared targets
/// (`request → authorize → mint → revoke`). Every mint/revoke is an audited event; the
/// grant itself is persisted via [`CredentialGrantRepository`]. The secret is delivered
/// out-of-band (see [`MintedCredential`]); it must never reach the model, its
/// checkpoints, or the event log.
pub trait CredentialBroker {
    /// Mint a credential for `run_id` against `target` (action + scope). Returns the
    /// grant (opaque handle) and the materialized secret for out-of-band delivery.
    async fn mint(
        &self,
        run: RunId,
        target: &str,
        action: &str,
        scope: &str,
    ) -> DomainResult<MintedCredential>;
    /// Revoke a previously minted grant, marking it `Revoked`.
    async fn revoke(&self, run: RunId, grant_id: &str) -> DomainResult<()>;
    /// Revoke every active grant for a run (terminal teardown). Idempotent: a run with
    /// no active grants is a no-op.
    async fn revoke_run(&self, run: RunId) -> DomainResult<()>;
}

/// Persistence for minted credential grants (`PLAN.md` §11.3). The materialized secret
/// is never written here — only the grant metadata.
pub trait CredentialGrantRepository {
    async fn create(&self, grant: &CredentialGrant) -> DomainResult<CredentialGrant>;
    async fn get(&self, id: &str) -> DomainResult<Option<CredentialGrant>>;
    /// All grants for a run whose TTL has not lapsed and which aren't revoked.
    async fn list_active(&self, run_id: RunId) -> DomainResult<Vec<CredentialGrant>>;
    /// Mark a grant `Revoked` (and stamp `revoked_at`). Idempotent.
    async fn revoke(&self, id: &str) -> DomainResult<CredentialGrant>;
    /// Hard-delete every grant for a run (retention/deletion, §11.7).
    async fn delete_run(&self, run_id: RunId) -> DomainResult<()>;
}

pub trait Notifier {
    async fn notify(&self, message: &str) -> DomainResult<()>;
}

/// The outcome of an atomic budget `consume` (`PLAN.md` §12). The counter is the
/// single serialization point for every model call in a run, so parallel subagent
/// fan-out cannot overshoot the cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsumeOutcome {
    /// The call is within budget; `consumed` is the running total.
    Ok { consumed: u64 },
    /// The call would exceed `cap` (the increment was rolled back).
    Exceeded { cap: u64, consumed: u64 },
}

/// Durable per-run budget cap (`PLAN.md` §12). The real-time consumed count is
/// authoritative in the [`BudgetCounter`]; this repository holds the cap that the
/// counter is seeded from and that `consume` enforces against.
pub trait BudgetRepository {
    /// Persist (upsert) the run's budget cap.
    async fn save(&self, budget: &Budget) -> DomainResult<Budget>;
    /// The run's budget, or `None` when it has none.
    async fn get(&self, run_id: RunId) -> DomainResult<Option<Budget>>;
    /// Hard-delete a run's budget (retention/deletion, §11.7).
    async fn delete(&self, run_id: RunId) -> DomainResult<()>;
}

/// The atomic, real-time budget counter (`PLAN.md` §12) — the authoritative hard-stop
/// enforcement point. Backed by Redis `INCRBY` (via a Lua script for atomicity) in
/// production, with an in-memory fallback for single-replica dev. Keyed by `run_id`,
/// so every model call (including subagent fan-out) hits the same counter.
pub trait BudgetCounter {
    /// Atomically consume `tokens` from the run's counter against `cap`. Returns
    /// `Ok` when within budget, `Exceeded` otherwise (the increment is rolled back).
    async fn consume(&self, run_id: RunId, cap: u64, tokens: u64) -> DomainResult<ConsumeOutcome>;
    /// The remaining allowance (`cap - consumed`), for the harness fast-path.
    async fn remaining(&self, run_id: RunId, cap: u64) -> DomainResult<u64>;
    /// Zero the run's counter (fresh run seed, or preemptive revocation mid-run).
    async fn reset(&self, run_id: RunId) -> DomainResult<()>;
}

/// Idempotency / dedup / replay cache for triggers (`PLAN.md` §11.5). The trigger's
/// `dedup_key` (and a per-source cooldown key) are recorded here with a TTL window; a
/// replayed event maps to the one already-accepted run instead of fanning out.
pub trait DedupRepository {
    /// Atomically record `key` for `ttl`. Returns `true` when `key` was already
    /// present (a duplicate/replay), `false` when newly recorded. `ttl` is the window
    /// the key is remembered for.
    async fn check_and_record(&self, key: &str, ttl: std::time::Duration) -> DomainResult<bool>;
}
