//! Core domain entities. Infrastructure-free: no framework or storage types.

use std::time::SystemTime;

use serde::{Deserialize, Serialize};

/// Opaque identifiers (newtypes so they cannot be mixed up).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TenantId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RunId(pub u64);

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
    pub tenant: TenantId,
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
}

/// Result-review decision (see `PLAN.md` §10).
#[derive(Debug, Clone)]
pub struct ReviewDecision {
    pub run_id: RunId,
    pub verdict: ReviewVerdict,
    pub feedback: String,
    pub reviewed_at: SystemTime,
}

/// A git repository an agent can clone and work on, with an optional access token.
///
/// The read token is stored **encrypted** (`token_ciphertext`); the domain never
/// sees the plaintext — encryption/decryption is an adapter concern (see `PLAN.md`
/// §10 "repo-scoped credential"). The write token (for opening PRs/MRs) is stored
/// as a Kubernetes Secret *reference* (name + key), like the LLM provider API keys,
/// so the plaintext write credential never enters the database or the sandbox.
#[derive(Debug, Clone)]
pub struct GitRepo {
    pub id: String,
    pub name: String,
    pub url: String,
    pub token_ciphertext: Option<String>,
    /// Kubernetes Secret holding the write token (used to open PRs/MRs).
    pub write_token_secret: Option<String>,
    /// Key within that Secret holding the write token.
    pub write_token_key: Option<String>,
    pub enabled: bool,
}
