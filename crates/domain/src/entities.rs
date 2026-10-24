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
}

/// Result-review decision (see `PLAN.md` §10).
#[derive(Debug, Clone)]
pub struct ReviewDecision {
    pub run_id: RunId,
    pub verdict: ReviewVerdict,
    pub feedback: String,
    pub reviewed_at: SystemTime,
}
