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
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub runs: Vec<SessionRun>,
}

/// Request body for `POST /v1/runs`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateRunRequest {
    pub task_spec: String,
    pub kind: RunKind,
    #[serde(default = "default_max_concurrency")]
    pub max_concurrency: u32,
    #[serde(default)]
    pub actor: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// Select an agent template; its configured model is used unless `model` is set.
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
}

fn default_max_concurrency() -> u32 {
    1
}

/// Response body for `POST /v1/runs` and the runs list (`GET /v1/runs`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunResponse {
    pub id: u64,
    pub kind: RunKind,
    pub state: RunState,
    pub trigger: String,
    pub actor: Option<String>,
    pub model: Option<String>,
    pub repo: Option<String>,
    pub language: Option<String>,
    pub session_id: Option<String>,
    pub task_spec: String,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

impl From<&crate::entities::Run> for RunResponse {
    fn from(run: &crate::entities::Run) -> Self {
        RunResponse {
            id: run.id.0,
            kind: run.kind,
            state: run.state,
            trigger: run.trigger.clone(),
            actor: run.actor.clone(),
            model: run.model.clone(),
            repo: run.repo.clone(),
            language: run.language.clone(),
            session_id: run.session_id.clone(),
            task_spec: run.task_spec.clone(),
            created_at: to_iso(run.created_at),
            started_at: run.started_at.map(to_iso),
            finished_at: run.finished_at.map(to_iso),
        }
    }
}

/// Project a lifecycle run into the session detail's display row. Fields that a
/// bare run doesn't carry (agent identity, cost) are left as placeholders; they'll
/// be populated as the harness reports events (PLAN.md §9). Lifecycle timing is
/// carried through so the display row can be kept in sync.
impl From<&crate::entities::Run> for SessionRun {
    fn from(run: &crate::entities::Run) -> Self {
        SessionRun {
            id: format!("run-{}", run.id.0),
            kind: run.kind,
            state: run.state,
            agent_id: String::new(),
            agent_name: "Default agent".into(),
            task_spec: run.task_spec.clone(),
            created_at: to_iso(run.created_at),
            started_at: run.started_at.map(to_iso),
            finished_at: run.finished_at.map(to_iso),
            cost_usd: None,
        }
    }
}

/// Format a [`std::time::SystemTime`] as an RFC-3339 UTC timestamp (seconds).
fn to_iso(t: std::time::SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(t)
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Current time as an RFC-3339 UTC timestamp (seconds) — used for new session rows.
pub fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// A fresh, unique session id.
pub fn new_session_id() -> String {
    format!("sess-{}", uuid::Uuid::new_v4())
}
