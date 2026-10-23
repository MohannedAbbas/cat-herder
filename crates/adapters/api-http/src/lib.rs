//! HTTP adapter (Axum): exposes the control-plane REST API (see `PLAN.md` §14).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};

use cat_herder_domain::api::{
    AgentTemplate, CreateRunRequest, RunResponse, Session, SessionRun, SessionSummary,
};
use cat_herder_domain::entities::{Run, RunId, RunState, TenantId};
use cat_herder_domain::ports::AgentRuntime;
use cat_herder_runtime_agent_sandbox::SandboxRuntime;

mod mock;

/// Application state: the data the handlers serve. In-memory mock data by default;
/// will be backed by Postgres repositories later.
#[derive(Clone)]
pub struct AppState {
    pub agents: Vec<AgentTemplate>,
    pub sessions: Vec<Session>,
    /// Optional execution runtime (present in the real binary, absent in tests/mock).
    pub runtime: Option<SandboxRuntime>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            agents: mock::agents(),
            sessions: mock::sessions(),
            runtime: None,
        }
    }
}

/// Build the public router.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/v1/runs", post(create_run))
        .route("/v1/sessions", get(list_sessions))
        .route("/v1/sessions/{id}", get(get_session))
        .route("/v1/agents", get(list_agents))
        .route("/v1/agents/{id}", get(get_agent))
        .with_state(state)
}

// --- Handlers -------------------------------------------------------------

static NEXT_RUN_ID: AtomicU64 = AtomicU64::new(1);

/// Accept a task and spawn a Sandbox for it via the execution runtime.
async fn create_run(
    State(state): State<AppState>,
    Json(req): Json<CreateRunRequest>,
) -> Result<(StatusCode, Json<RunResponse>), (StatusCode, String)> {
    let Some(runtime) = &state.runtime else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "execution runtime not configured".into(),
        ));
    };

    let run = Run {
        id: RunId(NEXT_RUN_ID.fetch_add(1, Ordering::Relaxed)),
        tenant: TenantId(1), // placeholder until multi-tenancy is wired
        kind: req.kind,
        state: RunState::Queued,
        max_concurrency: req.max_concurrency,
        created_at: SystemTime::now(),
    };

    runtime
        .spawn(&run)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok((
        StatusCode::CREATED,
        Json(RunResponse {
            id: run.id.0,
            kind: run.kind,
            state: run.state,
        }),
    ))
}

async fn list_sessions(State(state): State<AppState>) -> Json<Vec<SessionSummary>> {
    Json(state.sessions.iter().map(summarize).collect())
}

async fn get_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Session>, StatusCode> {
    state
        .sessions
        .iter()
        .find(|s| s.id == id)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn list_agents(State(state): State<AppState>) -> Json<Vec<AgentTemplate>> {
    Json(state.agents)
}

async fn get_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<AgentTemplate>, StatusCode> {
    state
        .agents
        .iter()
        .find(|a| a.id == id)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

// --- Helpers --------------------------------------------------------------

fn summarize(session: &Session) -> SessionSummary {
    SessionSummary {
        id: session.id.clone(),
        title: session.title.clone(),
        created_at: session.created_at.clone(),
        updated_at: session.updated_at.clone(),
        run_count: session.runs.len(),
        status: aggregate_status(&session.runs),
    }
}

/// Most-active run state wins (mirrors `ui/src/api/mock.ts`).
fn aggregate_status(runs: &[SessionRun]) -> RunState {
    const PRIORITY: [RunState; 14] = [
        RunState::Running,
        RunState::AwaitingInput,
        RunState::PodPending,
        RunState::Scheduled,
        RunState::Queued,
        RunState::BudgetPaused,
        RunState::Stuck,
        RunState::AwaitingReview,
        RunState::TimedOut,
        RunState::Failed,
        RunState::Rejected,
        RunState::Cancelled,
        RunState::Reviewed,
        RunState::Succeeded,
    ];
    PRIORITY
        .into_iter()
        .find(|s| runs.iter().any(|r| r.state == *s))
        .unwrap_or(RunState::Succeeded)
}
