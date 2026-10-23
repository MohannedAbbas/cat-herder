//! HTTP adapter (Axum): exposes the control-plane REST API (see `PLAN.md` §14).

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};

use cat_herder_application::start_run;
use cat_herder_domain::api::{
    AgentTemplate, CreateRunRequest, RunResponse, Session, SessionRun, SessionSummary,
};
use cat_herder_domain::entities::{RunId, RunState, TenantId};
use cat_herder_domain::ports::{RunRepository, RunRequest};
use cat_herder_runtime_agent_sandbox::SandboxRuntime;
use cat_herder_store_inmemory::InMemoryRunRepository;

mod mock;

/// Application state: the data the handlers serve. In-memory mock data by default;
/// will be backed by Postgres repositories later.
#[derive(Clone)]
pub struct AppState {
    pub agents: Vec<AgentTemplate>,
    pub sessions: Vec<Session>,
    pub repo: InMemoryRunRepository,
    /// Optional execution runtime (present in the real binary, absent in tests/mock).
    pub runtime: Option<SandboxRuntime>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            agents: mock::agents(),
            sessions: mock::sessions(),
            repo: InMemoryRunRepository::new(),
            runtime: None,
        }
    }
}

/// Build the public router.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/v1/runs", get(list_runs).post(create_run))
        .route("/v1/runs/{id}", get(get_run))
        .route("/v1/sessions", get(list_sessions))
        .route("/v1/sessions/{id}", get(get_session))
        .route("/v1/agents", get(list_agents))
        .route("/v1/agents/{id}", get(get_agent))
        .with_state(state)
}

// --- Run handlers ---------------------------------------------------------

/// Accept a task, orchestrate it through the lifecycle, and spawn a Sandbox.
async fn create_run(
    State(state): State<AppState>,
    Json(req): Json<CreateRunRequest>,
) -> Result<(StatusCode, Json<RunResponse>), (StatusCode, String)> {
    let runtime = state.runtime.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "execution runtime not configured".into(),
    ))?;

    let run_req = RunRequest {
        tenant: TenantId(1), // placeholder until multi-tenancy is wired
        kind: req.kind,
        task_spec: req.task_spec,
        dedup_key: String::new(),
        max_concurrency: req.max_concurrency,
    };

    let run = start_run(run_req, runtime, &state.repo)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok((StatusCode::CREATED, Json(RunResponse::from(&run))))
}

async fn list_runs(State(state): State<AppState>) -> Json<Vec<RunResponse>> {
    let runs = state.repo.list_by_tenant(TenantId(1)).await.unwrap_or_default();
    Json(runs.iter().map(RunResponse::from).collect())
}

async fn get_run(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> Result<Json<RunResponse>, StatusCode> {
    let run = state
        .repo
        .get(RunId(id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(RunResponse::from(&run)))
}

// --- Session / agent handlers --------------------------------------------

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
