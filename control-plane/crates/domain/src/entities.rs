//! Core domain entities. Infrastructure-free: no framework or storage types.

use std::time::SystemTime;

use serde::{Deserialize, Serialize};

/// Opaque identifier (newtype so it cannot be mixed up with other ids).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RunId(pub u64);

/// LLM usage reported by the harness when a run finishes (token counts).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunKind {
    Agent,
    Workflow,
}

/// Lifecycle state — the single source of truth (see `PLAN.md` §8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Queued,
    Scheduled,
    PodPending,
    Running,
    AwaitingInput,
    BudgetPaused,
    Succeeded,
    AwaitingReview,
    Reviewed,
    Rejected,
    Failed,
    TimedOut,
    Cancelled,
    Stuck,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewVerdict {
    Approved,
    Rejected,
    ChangesRequested,
}

/// A single execution (agent or workflow).
#[derive(Debug, Clone)]
pub struct Run {
    pub id: RunId,
    pub kind: RunKind,
    pub state: RunState,
    pub task_spec: String,
    /// The session this run belongs to (auto-created on `start_run`).
    pub session_id: Option<String>,
    pub max_concurrency: u32,
    pub created_at: SystemTime,
    /// Trigger context — where the run came from and who/what requested it.
    pub trigger: String,
    pub actor: Option<String>,
    /// Agent/model + repository + language context (optional, trigger-provided).
    pub model: Option<String>,
    pub repo: Option<String>,
    pub language: Option<String>,
    /// Lifecycle timing (set as the run transitions through the state machine).
    pub started_at: Option<SystemTime>,
    pub finished_at: Option<SystemTime>,
    /// PR URL, set when the control plane opens a pull request from the agent's diff.
    pub pr_url: Option<String>,
    /// The agent's final markdown report, set from the harness `finished` event.
    pub report: Option<String>,
    /// LLM token usage reported by the harness (0 until the `finished` event).
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// A LangGraph checkpoint flushed by the harness and persisted opaquely by the control
/// plane (`PLAN.md` §6, `docs/design/checkpointing.md`). `seq` is the per-run monotonic
/// order assigned by the control plane (sole writer); `payload` is the serialized
/// `CheckpointTuple` (opaque — the control plane never parses LangGraph internals).
#[derive(Debug, Clone)]
pub struct Checkpoint {
    pub run_id: RunId,
    pub seq: i64,
    pub thread_id: String,
    pub checkpoint_ns: String,
    pub checkpoint_id: String,
    pub serde_type: String,
    pub payload: Vec<u8>,
    pub occurred_at: SystemTime,
}

/// Result-review decision (see `PLAN.md` §10).
#[derive(Debug, Clone)]
pub struct ReviewDecision {
    pub run_id: RunId,
    pub verdict: ReviewVerdict,
    pub feedback: String,
    pub reviewed_at: SystemTime,
}

/// Whether a human-in-the-loop approval has been decided, and how.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Rejected,
}

/// A durable human-in-the-loop approval request (see `PLAN.md` §10). `resume_token` is
/// the opaque LangGraph checkpoint id of the paused state — the control plane stores it
/// but never interprets it; `payload` is the pending tool call/question shown to the
/// human.
#[derive(Debug, Clone)]
pub struct ApprovalRequest {
    pub id: String,
    pub run_id: RunId,
    pub resume_token: String,
    pub payload: serde_json::Value,
    pub status: ApprovalStatus,
    pub response: Option<String>,
    pub created_at: SystemTime,
    pub responded_at: Option<SystemTime>,
}

/// A git repository an agent can clone and work on, with optional access tokens.
///
/// Both the read token (`token_ciphertext`) and the write token
/// (`write_token_ciphertext`, used to open PRs/MRs) are stored **encrypted**; the
/// domain never sees the plaintext — encryption/decryption is an adapter concern
/// (see `PLAN.md` §10 "repo-scoped credential").
#[derive(Debug, Clone)]
pub struct GitRepo {
    pub id: String,
    pub name: String,
    pub url: String,
    pub token_ciphertext: Option<String>,
    /// Encrypted write token (opens PRs/MRs against the repo's host).
    pub write_token_ciphertext: Option<String>,
    pub enabled: bool,
}

/// A per-run token budget (`PLAN.md` §12). `cap` is the authoritative token ceiling
/// (durable in Postgres); the real-time consumed count lives in the atomic budget
/// counter (Redis), and the final attribution rollup lands on the run's token usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    pub run_id: RunId,
    /// Token cap for the run. `0` means "no budget" (unbounded — enforcement skipped).
    pub cap: u64,
    pub updated_at: SystemTime,
}

/// Lifecycle of a minted credential grant (`PLAN.md` §11.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialGrantStatus {
    /// Minted and within its TTL.
    Active,
    /// Explicitly revoked (run end / policy / manual).
    Revoked,
    /// Lapsed past `expires_at` without explicit revocation.
    Expired,
}

/// A JIT-minted credential grant (`PLAN.md` §11.3).
///
/// Every mint is recorded as a `CredentialGrant`; `revoke` marks it `Revoked` (or the
/// TTL expires it). The materialized secret is **never stored here** — only metadata —
/// so the grants table holds no credentials at rest. `id` is the opaque handle
/// returned to the caller (`cred://…`); the secret itself is delivered out-of-band to
/// the requesting client (credential helper / sidecar), never to the model.
#[derive(Debug, Clone)]
pub struct CredentialGrant {
    pub id: String,
    pub run_id: RunId,
    /// The target the credential is scoped to (e.g. `github.com/org/repo`).
    pub target: String,
    /// The requested action (`clone`, `push`, `pr`, …).
    pub action: String,
    /// The requested scope (`read`, `write`).
    pub scope: String,
    pub status: CredentialGrantStatus,
    pub expires_at: SystemTime,
    pub created_at: SystemTime,
    pub revoked_at: Option<SystemTime>,
    /// The issuer's server-side identifier for this token (GitLab project-token id /
    /// Forgejo token id), used to revoke it server-side. `None` when the issuer has no
    /// per-token revocation (GitHub installation tokens lapse via TTL). Never the secret.
    pub token_id: Option<String>,
}
