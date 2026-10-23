//! API-facing read-model types (the JSON contract of the REST API, `PLAN.md` §14).
//!
//! These are the shapes returned by `GET /v1/sessions` and `GET /v1/agents`, and
//! mirror the UI's TypeScript types in `ui/src/types/domain.ts`.

use serde::{Deserialize, Serialize};

use crate::entities::{RunKind, RunState};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuntimeTier {
    Default,
    Gvisor,
    Kata,
}

/// A reusable agent configuration (maps to SandboxTemplate + harness config).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
    pub image: String,
    pub runtime_tier: RuntimeTier,
    pub model: String,
    pub tools: Vec<String>,
    pub max_concurrency: u32,
}

/// A single run as exposed in a session (richer than the lifecycle `Run`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRun {
    pub id: String,
    pub kind: RunKind,
    pub state: RunState,
    pub agent_id: String,
    pub agent_name: String,
    pub task_spec: String,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub cost_usd: Option<f64>,
}

/// Summary row for a session (list view).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub run_count: usize,
    pub status: RunState,
}

/// A session with its full set of runs (detail view).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub runs: Vec<SessionRun>,
}
