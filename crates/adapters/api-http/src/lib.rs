//! HTTP adapter (Axum): exposes the control-plane REST API (see `PLAN.md` §14).

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};

use cat_herder_application::{Outcome, complete_run, start_run};
use cat_herder_domain::api::{
    AgentTemplate, CreateRunRequest, RunResponse, Session, SessionRun, SessionSummary,
};
use cat_herder_domain::entities::{RunId, RunState, TenantId};
use cat_herder_domain::ports::{AgentRepository, RunRepository, RunRequest, SessionRepository};
use cat_herder_runtime_agent_sandbox::SandboxRuntime;

pub use repo::{AgentRepo, RunRepo, SessionRepo};

mod mcp;
mod repo;
mod webhook;

/// Application state: the data the handlers serve. Runs, agents, and sessions are
/// backed by their repositories (Postgres in production, in-memory fallback).
#[derive(Clone)]
pub struct AppState {
    pub agents: AgentRepo,
    pub sessions: SessionRepo,
    pub repo: RunRepo,
    /// Optional execution runtime (present in the real binary, absent in tests/mock).
    pub runtime: Option<SandboxRuntime>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            agents: AgentRepo::InMemory(Default::default()),
            sessions: SessionRepo::InMemory(Default::default()),
            repo: RunRepo::InMemory(Default::default()),
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
        .route("/v1/runs/{id}/events", post(post_run_event))
        .route("/v1/webhooks/generic", post(webhook_generic))
        .route("/mcp", post(mcp_handler))
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
        trigger: "api".into(),
        actor: req.actor,
        model: req.model,
        repo: req.repo,
        language: req.language,
    };

    let run = start_run(run_req, runtime, &state.repo, &state.sessions)
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

/// A harness-reported event (see `PLAN.md` §9). The only state-changing event today
/// is `finished` (advances the run to `succeeded`/`failed`); others are acknowledged.
#[derive(Debug, serde::Deserialize)]
pub struct RunEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(default)]
    pub status: Option<String>,
}

/// Ingest a harness-reported event for a run.
async fn post_run_event(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    Json(event): Json<RunEvent>,
) -> Result<Json<RunResponse>, (StatusCode, String)> {
    let run_id = RunId(id);
    let exists = state
        .repo
        .get(run_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "run not found".to_string()))?;

    match event.event_type.as_str() {
        "finished" => {
            let outcome = match event.status.as_deref() {
                Some("failed") => Outcome::Failed,
                _ => Outcome::Succeeded,
            };
            let run = complete_run(run_id, outcome, &state.repo, &state.sessions)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            Ok(Json(RunResponse::from(&run)))
        }
        // "started" and any other event type: acknowledged without a state change.
        _ => Ok(Json(RunResponse::from(&exists))),
    }
}

/// Accept an external webhook, normalize it, and trigger a run.
async fn webhook_generic(
    State(state): State<AppState>,
    Json(payload): Json<webhook::WebhookPayload>,
) -> Result<(StatusCode, Json<RunResponse>), (StatusCode, String)> {
    let runtime = state.runtime.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "execution runtime not configured".into(),
    ))?;

    let run = start_run(webhook::normalize(payload), runtime, &state.repo, &state.sessions)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok((StatusCode::CREATED, Json(RunResponse::from(&run))))
}

/// MCP endpoint (Streamable HTTP): JSON-RPC 2.0 in, JSON-RPC response out.
async fn mcp_handler(
    State(state): State<AppState>,
    Json(request): Json<mcp::JsonRpcRequest>,
) -> Json<serde_json::Value> {
    Json(mcp::handle(request, &state).await)
}

async fn list_sessions(State(state): State<AppState>) -> Json<Vec<SessionSummary>> {
    let sessions = state.sessions.list().await.unwrap_or_default();
    Json(sessions.iter().map(summarize).collect())
}

async fn get_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Session>, StatusCode> {
    state
        .sessions
        .get(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn list_agents(State(state): State<AppState>) -> Json<Vec<AgentTemplate>> {
    Json(state.agents.list().await.unwrap_or_default())
}

async fn get_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<AgentTemplate>, StatusCode> {
    state
        .agents
        .get(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
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
