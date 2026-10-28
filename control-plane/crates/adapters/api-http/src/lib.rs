//! HTTP adapter (Axum): exposes the control-plane REST API (see `PLAN.md` §14).

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::sse::{Event as SseEvent, KeepAlive, Sse},
    routing::{get, post, put},
};
use futures::StreamExt;
use tokio::sync::RwLock;

use cat_herder_application::{Completion, Outcome, complete_run, respond_approval, start_run};
use cat_herder_credential_broker::CredentialBrokerService;
use cat_herder_domain::api::{
    AgentTemplate, CreateRunRequest, EventListResponse, EventResponse, RunResponse, Session,
    SessionRun, SessionSummary, repo_identity_from_url,
};
use cat_herder_domain::entities::{ApprovalStatus, Run, RunId, RunState, Usage};
use cat_herder_domain::events::{Event, event_type};
use cat_herder_domain::ports::{
    AgentRepository, AgentRuntime, ApprovalRepository, EventBus, EventRepository,
    GitRepoRepository, RunRepository, RunRequest, SessionRepository, SettingRepository,
};
use cat_herder_runtime_agent_sandbox::SandboxRuntime;
use cat_herder_store_inmemory::{
    InMemoryEventRepository, InMemoryRunRepository, InMemoryTransitionRepository,
};

pub use auth::Auth;
pub use bus::EventBusImpl;
pub use crypto::AesGcmCipher;
pub use grpc::{ControlLinks, grpc_serve, grpc_server};
pub use key_store::KeySecretStore;
pub use oauth::{GITHUB, GITLAB, GitHubOAuth, GitLabOAuth, OAuthConfig};
pub use pr::{FileChange, PrClient, PrSpec, default_host_api_bases};
pub use repo::{
    AgentRepo, ApprovalRepo, AuditRepo, CheckpointRepo, CredentialGrantRepo, EventRepo,
    GitRepoRepo, ModelRepo, ProviderRepo, RunRepo, SessionRepo, SettingRepo, ToolRepo,
    TransitionRepo,
};

mod admin;
mod auth;
mod bus;
mod crypto;
mod grpc;
mod key_store;
mod mcp;
mod oauth;
mod pr;
mod repo;
mod webhook;

/// Settings key for the shared webhook secret.
pub const WEBHOOK_SECRET_KEY: &str = "webhook_secret";

/// Settings key for the supported git hosts (a JSON array of `{host,label}`).
pub const GIT_HOSTS_SETTING: &str = "git_hosts";

/// The concrete credential broker wired into the application: the [`CredentialBroker`]
/// service over the in-memory/Postgres grant + event repositories and the fan-out bus.
pub type Broker = CredentialBrokerService<CredentialGrantRepo, EventRepo, EventBusImpl>;

/// Application state: the data the handlers serve. Runs, agents, sessions,
/// providers, and models are backed by their repositories (Postgres in production,
/// in-memory fallback).
#[derive(Clone)]
pub struct AppState {
    pub agents: AgentRepo,
    pub providers: ProviderRepo,
    pub models: ModelRepo,
    pub tools: ToolRepo,
    pub git_repos: GitRepoRepo,
    pub sessions: SessionRepo,
    pub repo: RunRepo,
    pub events: EventRepo,
    pub transitions: TransitionRepo,
    pub checkpoints: CheckpointRepo,
    pub approvals: ApprovalRepo,
    pub grants: CredentialGrantRepo,
    /// The JIT credential broker (`PLAN.md` §11.3). When no GitHub App is configured,
    /// its issuer map is empty and `mint` fails gracefully (the runtime falls back to a
    /// registered repo token or none).
    pub broker: Broker,
    pub bus: EventBusImpl,
    pub audit: AuditRepo,
    pub settings: SettingRepo,
    pub auth: Auth,
    /// Optional GitHub OAuth SSO provider (present when its env vars are set).
    pub github_oauth: Option<GitHubOAuth>,
    /// Optional GitLab OAuth SSO provider (present when its env vars are set).
    pub gitlab_oauth: Option<GitLabOAuth>,
    /// Dev-only mock OAuth mode. When `Some(username)`, both GitHub and GitLab SSO are
    /// reported as enabled and their `/login` routes skip the real provider redirect,
    /// issuing an admin session directly for `username`. Set via `CAT_HERDER_OAUTH_MOCK`
    /// (+ `CAT_HERDER_OAUTH_MOCK_USERNAME`); must be off in production.
    pub mock_oauth: Option<String>,
    /// Optional execution runtime (present in the real binary, absent in tests/mock).
    pub runtime: Option<SandboxRuntime>,
    /// Live harness control links (`run_id -> outbound sender`) for HITL signals (§10).
    pub control_links: Arc<ControlLinks>,
    /// Encryption cipher for git tokens (always present; key lives in a Secret and is
    /// rotated via the admin API).
    pub cipher: Arc<RwLock<AesGcmCipher>>,
    /// Secret-backed encryption-key store (present in the real binary; absent in tests).
    pub key_store: Option<Arc<KeySecretStore>>,
    /// Host-aware PR/MR client (stateless; the write token is read per-run from the
    /// repo's referenced Secret). Opens a PR from the agent's diff when a run finishes.
    pub pr_client: Arc<PrClient>,
}

impl Default for AppState {
    fn default() -> Self {
        // Share the in-memory run + event stores with the transition repository, so a
        // lifecycle transition is visible through the run/event repos — matching the
        // real composition root (see `bin/cat-herder/src/main.rs::Repos::in_memory`).
        let run = InMemoryRunRepository::new();
        let events = InMemoryEventRepository::default();
        let transitions = InMemoryTransitionRepository::new(run.clone(), events.clone());
        let grants = CredentialGrantRepo::InMemory(Default::default());
        let bus = EventBusImpl::Noop;
        // No GitHub App configured by default, so the broker has an empty issuer map
        // and `mint` fails gracefully.
        let broker = CredentialBrokerService::new(
            Default::default(),
            grants.clone(),
            EventRepo::InMemory(events.clone()),
            bus.clone(),
        );
        Self {
            agents: AgentRepo::InMemory(Default::default()),
            providers: ProviderRepo::InMemory(Default::default()),
            models: ModelRepo::InMemory(Default::default()),
            tools: ToolRepo::InMemory(Default::default()),
            git_repos: GitRepoRepo::InMemory(Default::default()),
            sessions: SessionRepo::InMemory(Default::default()),
            repo: RunRepo::InMemory(run),
            events: EventRepo::InMemory(events),
            transitions: TransitionRepo::InMemory(transitions),
            checkpoints: CheckpointRepo::InMemory(Default::default()),
            approvals: ApprovalRepo::InMemory(Default::default()),
            grants,
            broker,
            bus,
            audit: AuditRepo::InMemory(Default::default()),
            settings: SettingRepo::InMemory(Default::default()),
            auth: Auth::new("admin", "admin"),
            github_oauth: None,
            gitlab_oauth: None,
            mock_oauth: None,
            runtime: None,
            control_links: Arc::new(ControlLinks::default()),
            cipher: Arc::new(RwLock::new(
                AesGcmCipher::generate().expect("rng available"),
            )),
            key_store: None,
            pr_client: Arc::new(PrClient::from_env()),
        }
    }
}

/// Build the public router.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/v1/runs", get(list_runs).post(create_run))
        .route("/v1/runs/{id}", get(get_run))
        .route(
            "/v1/runs/{id}/events",
            get(get_run_events).post(post_run_event),
        )
        .route("/v1/runs/{id}/events/stream", get(stream_run_events))
        .route("/v1/approvals/{id}/respond", post(respond_approval_handler))
        .route("/v1/webhooks/generic", post(webhook_generic))
        .route("/mcp", post(mcp_handler))
        .route("/v1/sessions", get(list_sessions))
        .route("/v1/sessions/{id}", get(get_session))
        .route("/v1/agents", get(list_agents))
        .route("/v1/agents/{id}", get(get_agent))
        // auth
        .route("/v1/auth/login", post(admin::login))
        .route("/v1/auth/logout", post(admin::logout))
        .route("/v1/auth/me", get(admin::me))
        .route("/v1/auth/sso-providers", get(admin::sso_providers))
        .route("/v1/auth/github/login", get(admin::github_login))
        .route("/v1/auth/github/callback", get(admin::github_callback))
        .route("/v1/auth/gitlab/login", get(admin::gitlab_login))
        .route("/v1/auth/gitlab/callback", get(admin::gitlab_callback))
        // admin
        .route(
            "/v1/admin/agents/{id}",
            put(admin::upsert_agent).delete(admin::delete_agent),
        )
        .route("/v1/admin/providers", get(admin::list_providers))
        .route(
            "/v1/admin/providers/{id}",
            put(admin::upsert_provider).delete(admin::delete_provider),
        )
        .route("/v1/admin/models", get(admin::list_models))
        .route(
            "/v1/admin/models/{id}",
            put(admin::upsert_model).delete(admin::delete_model),
        )
        .route("/v1/admin/tools", get(admin::list_tools))
        .route(
            "/v1/admin/tools/{id}",
            put(admin::upsert_tool).delete(admin::delete_tool),
        )
        .route(
            "/v1/admin/repos",
            get(admin::list_repos)
                .put(admin::upsert_repo)
                .delete(admin::delete_repo),
        )
        .route("/v1/admin/git-hosts", get(admin::list_git_hosts))
        .route(
            "/v1/admin/settings/webhook-secret",
            get(admin::get_webhook_secret).post(admin::regenerate_webhook_secret),
        )
        .route(
            "/v1/admin/settings/encryption-key/rotate",
            post(admin::rotate_encryption_key),
        )
        .route("/v1/admin/audit", get(admin::list_audit))
        .with_state(state)
}

// --- Run handlers ---------------------------------------------------------

/// Resolve the model for a run: an explicit `model` wins, else the selected agent's
/// model. Returns `None` when neither is provided (models are agent-specific).
async fn resolve_model(
    state: &AppState,
    explicit: Option<String>,
    agent_id: Option<String>,
) -> Result<Option<String>, (StatusCode, String)> {
    match explicit {
        Some(m) => Ok(Some(m)),
        None => match agent_id {
            Some(id) => {
                let agent = state
                    .agents
                    .get(&id)
                    .await
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
                    .ok_or((StatusCode::NOT_FOUND, format!("agent '{id}' not found")))?;
                Ok(Some(agent.model).filter(|m| !m.is_empty()))
            }
            None => Ok(None),
        },
    }
}

/// Accept a task, orchestrate it through the lifecycle, and spawn a Sandbox.
async fn create_run(
    State(state): State<AppState>,
    Json(req): Json<CreateRunRequest>,
) -> Result<(StatusCode, Json<RunResponse>), (StatusCode, String)> {
    let runtime = state.runtime.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "execution runtime not configured".into(),
    ))?;

    // Models are agent-specific; a run needs a resolvable model.
    let model = resolve_model(&state, req.model, req.agent_id)
        .await?
        .ok_or((
            StatusCode::BAD_REQUEST,
            "agent_id or model is required".to_string(),
        ))?;

    let run_req = RunRequest {
        kind: req.kind,
        task_spec: req.task_spec,
        dedup_key: String::new(),
        max_concurrency: req.max_concurrency,
        trigger: "api".into(),
        actor: req.actor,
        model: Some(model),
        repo: req.repo,
        language: req.language,
    };

    let run = start_run(
        run_req,
        runtime,
        &state.repo,
        &state.sessions,
        &state.transitions,
        &state.bus,
        &state.broker,
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok((StatusCode::CREATED, Json(RunResponse::from(&run))))
}

async fn list_runs(State(state): State<AppState>) -> Json<Vec<RunResponse>> {
    let runs = state.repo.list_all().await.unwrap_or_default();
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

/// A harness-reported event (see `PLAN.md` §9). State-changing events: `finished`
/// (advances the run to `succeeded`/`failed`) and `stuck` (advances to `stuck`);
/// others are acknowledged.
#[derive(Debug, serde::Deserialize)]
pub struct RunEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(default)]
    pub status: Option<String>,
    /// The agent's final report (used as the PR body).
    #[serde(default)]
    pub report: Option<String>,
    /// Changed files reported by the harness (used to open a PR).
    #[serde(default)]
    pub changes: Option<Vec<FileChange>>,
    /// LLM token usage reported by the harness on completion.
    #[serde(default)]
    pub usage: Option<Usage>,
}

/// Append a harness-sourced (agent projection) event to the log and fan it out.
/// Best-effort, mirroring the application's `emit_event`.
async fn append_agent_event(state: &AppState, run_id: RunId, event: Event) {
    match state.events.append(run_id, &event).await {
        Ok(stored) => {
            if let Err(e) = state.bus.publish(&stored).await {
                eprintln!("event publish failed for run {}: {e}", run_id.0);
            }
        }
        Err(e) => eprintln!("agent event append failed for run {}: {e}", run_id.0),
    }
}

/// Query params for replaying a run's event log.
#[derive(Debug, serde::Deserialize)]
struct EventsQuery {
    #[serde(default)]
    after_seq: i64,
    #[serde(default = "default_events_limit")]
    limit: i64,
}

fn default_events_limit() -> i64 {
    100
}

/// Replay a run's canonical event log, paginated by `after_seq` (`PLAN.md` §9).
async fn get_run_events(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    Query(q): Query<EventsQuery>,
) -> Result<Json<EventListResponse>, (StatusCode, String)> {
    let run_id = RunId(id);
    state
        .repo
        .get(run_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "run not found".to_string()))?;

    let limit = q.limit.clamp(1, 1000);
    let events = state
        .events
        .list(run_id, q.after_seq, limit)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let next_after = events.last().map(|e| e.seq).unwrap_or(q.after_seq);
    Ok(Json(EventListResponse {
        events: events.iter().map(EventResponse::from).collect(),
        next_after,
    }))
}

/// Live-tail a run's event log as an SSE stream (`PLAN.md` §9). Subscribes to the
/// NATS `run.<id>.events` subject and forwards each appended event as it lands. Returns
/// `501` when NATS is not configured — clients fall back to polling `get_run_events`.
async fn stream_run_events(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> Result<
    Sse<impl futures::Stream<Item = Result<SseEvent, std::convert::Infallible>>>,
    (StatusCode, String),
> {
    let run_id = RunId(id);
    state
        .repo
        .get(run_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "run not found".to_string()))?;

    let stream = state.bus.subscribe(run_id).await.ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "event stream requires NATS".to_string(),
    ))?;

    let sse = stream.filter_map(move |item| async move {
        match item {
            Ok(ev) => {
                let id = ev.seq.to_string();
                let json = serde_json::to_string(&ev).unwrap_or_default();
                Some(Ok(SseEvent::default().id(id).data(json)))
            }
            Err(e) => {
                eprintln!("SSE stream error for run {}: {e}", run_id.0);
                None
            }
        }
    });

    Ok(Sse::new(sse).keep_alive(KeepAlive::default()))
}

/// Request body for `POST /v1/approvals/{id}/respond` (`PLAN.md` §10).
#[derive(Debug, serde::Deserialize)]
pub struct RespondApprovalRequest {
    #[serde(default)]
    pub approve: bool,
    #[serde(default)]
    pub response: Option<String>,
}

/// Record a human's decision on an approval and signal the harness to resume (approve)
/// or cancel (reject).
async fn respond_approval_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<RespondApprovalRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // 404 for unknown, 409 for an already-decided approval.
    let existing = state
        .approvals
        .get(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "approval not found".to_string()))?;
    if existing.status != ApprovalStatus::Pending {
        return Err((StatusCode::CONFLICT, "approval already decided".to_string()));
    }

    let approval = respond_approval(
        &id,
        req.approve,
        req.response.clone(),
        &state.approvals,
        &state.events,
        &state.repo,
        &state.sessions,
        &state.transitions,
        &state.bus,
        &state.broker,
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Signal the live harness to resume (or cancel). If the link is gone (the pod died),
    // this is a no-op — the run is already persisted and #6's reconcile will re-create
    // the pod to resume from the checkpoint.
    let (signal, response) = if approval.status == ApprovalStatus::Approved {
        ("resume", approval.response.clone())
    } else {
        ("cancel", None)
    };
    state
        .control_links
        .send_signal(approval.run_id, signal, response)
        .await;

    Ok(Json(serde_json::json!({
        "id": approval.id,
        "run_id": approval.run_id.0,
        "status": approval.status,
        "response": approval.response,
    })))
}

/// Ingest a harness-reported event (shared by the HTTP and gRPC adapters). Records the
/// agent-projection event, then — for `finished`/`stuck` — advances the run's lifecycle,
/// reclaims the sandbox, and (on success with a diff) opens a PR.
pub(crate) async fn ingest_harness_event(
    state: &AppState,
    run_id: RunId,
    event_type: &str,
    status: Option<String>,
    report: Option<String>,
    changes: Option<Vec<FileChange>>,
    usage: Option<Usage>,
) -> Result<RunResponse, (StatusCode, String)> {
    let exists = state
        .repo
        .get(run_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "run not found".to_string()))?;

    match event_type {
        "finished" => {
            let outcome = match status.as_deref() {
                Some("failed") => Outcome::Failed,
                _ => Outcome::Succeeded,
            };

            // Record the harness's own event (agent projection) before the lifecycle
            // transition it triggers.
            let agent_event = Event::new(
                run_id,
                event_type::AGENT_FINISHED,
                None,
                serde_json::json!({
                    "status": status.clone(),
                    "report": report.clone(),
                    "changes": changes.clone(),
                    "usage": usage,
                }),
            );
            append_agent_event(state, run_id, agent_event).await;

            let run = complete_run(
                run_id,
                outcome,
                Completion {
                    report: report.clone(),
                    usage,
                },
                &state.repo,
                &state.sessions,
                &state.transitions,
                &state.bus,
                &state.broker,
            )
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

            // The run's outcome and report are already persisted, so the sandbox has
            // nothing left to do: reclaim it now rather than leave its pod running
            // indefinitely. Deleting the Sandbox cascades (owner ref) to its pod.
            // Best-effort — a failure here must not fail the already-recorded run.
            if let Some(runtime) = &state.runtime
                && let Err(e) = runtime.terminate(run_id).await
            {
                eprintln!("failed to terminate sandbox for run {}: {e}", run_id.0);
            }

            // On success with a diff, open a PR in the background (the write token
            // lives here in the control plane, never in the sandbox).
            if outcome == Outcome::Succeeded {
                maybe_open_pr(state, &run, report, changes).await;
            }

            Ok(RunResponse::from(&run))
        }
        // The harness's own stuck watchdog reports `stuck` when the agent stops making
        // progress: mark the run terminal and reclaim the sandbox (the liveness probe is
        // the kubelet-side backstop if the harness itself is too wedged to report).
        "stuck" => {
            append_agent_event(
                state,
                run_id,
                Event::new(
                    run_id,
                    event_type::AGENT_STUCK,
                    None,
                    serde_json::Value::Null,
                ),
            )
            .await;

            let run = complete_run(
                run_id,
                Outcome::Stuck,
                Completion::default(),
                &state.repo,
                &state.sessions,
                &state.transitions,
                &state.bus,
                &state.broker,
            )
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

            if let Some(runtime) = &state.runtime
                && let Err(e) = runtime.terminate(run_id).await
            {
                eprintln!("failed to terminate sandbox for run {}: {e}", run_id.0);
            }

            Ok(RunResponse::from(&run))
        }
        // "started" is acknowledged and recorded; any other event type is acknowledged
        // without a state change.
        "started" => {
            append_agent_event(
                state,
                run_id,
                Event::new(
                    run_id,
                    event_type::AGENT_STARTED,
                    None,
                    serde_json::Value::Null,
                ),
            )
            .await;
            Ok(RunResponse::from(&exists))
        }
        _ => Ok(RunResponse::from(&exists)),
    }
}

/// HTTP handler for `POST /v1/runs/{id}/events`.
async fn post_run_event(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    Json(event): Json<RunEvent>,
) -> Result<Json<RunResponse>, (StatusCode, String)> {
    let RunEvent {
        event_type,
        status,
        report,
        changes,
        usage,
    } = event;
    let run = ingest_harness_event(
        &state,
        RunId(id),
        &event_type,
        status,
        report,
        changes,
        usage,
    )
    .await?;
    Ok(Json(run))
}

/// Open a PR/MR from the agent's diff, if the repo has a write token and changes
/// exist. The write token is decrypted here (control-plane only, never the sandbox).
async fn maybe_open_pr(
    state: &AppState,
    run: &Run,
    report: Option<String>,
    changes: Option<Vec<FileChange>>,
) {
    let Some(changes) = changes else { return };
    if changes.is_empty() {
        return;
    }
    let Some(repo_url) = run.repo.as_deref() else {
        return;
    };

    // Resolve the repo identity and its encrypted write token.
    let Some(identity) = repo_identity_from_url(repo_url) else {
        return;
    };
    let repo_record = match state.git_repos.get(&identity.id()).await {
        Ok(Some(r)) => r,
        _ => return,
    };
    let Some(write_token_ciphertext) = repo_record.write_token_ciphertext else {
        eprintln!("run {} has no write token; skipping PR", run.id.0);
        return;
    };

    // Decrypt the write token (control-plane only).
    let token = {
        let cipher = state.cipher.read().await;
        match cipher.decrypt(&write_token_ciphertext) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("failed to decrypt write token for run {}: {e}", run.id.0);
                return;
            }
        }
    };

    let host = identity.host;
    let path = identity.path;
    let report = report.unwrap_or_default();
    let task_spec = run.task_spec.clone();
    let run_id = run.id;
    let pr_client = state.pr_client.clone();
    let state = state.clone();

    tokio::spawn(async move {
        // Suffix the branch with a Unix timestamp so a DB reset (which reuses run
        // ids) can't collide with a stale branch from a previous incarnation.
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let head = format!("cat-herder/run-{}-{}", run_id.0, ts);
        let title = pr_title(&task_spec);
        let spec = PrSpec {
            head: &head,
            title: &title,
            body: &report,
            changes: &changes,
        };
        match pr_client.create_pr(&host, &token, &path, &spec).await {
            Ok(url) => {
                eprintln!("opened PR for run {}: {url}", run_id.0);
                if let Ok(Some(mut r)) = state.repo.get(run_id).await {
                    r.pr_url = Some(url);
                    let _ = state.repo.save(&r).await;
                }
            }
            Err(e) => eprintln!("PR creation failed for run {}: {e}", run_id.0),
        }
    });
}

/// Derive a short PR title from the task's first line.
fn pr_title(task_spec: &str) -> String {
    let first_line = task_spec.lines().next().map(str::trim).unwrap_or("");
    if first_line.is_empty() {
        "cat-herder run".to_string()
    } else {
        first_line.chars().take(80).collect()
    }
}

/// Accept an external webhook, normalize it, and trigger a run. Requires the
/// configured webhook secret in the `X-Webhook-Secret` header.
async fn webhook_generic(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<webhook::WebhookPayload>,
) -> Result<(StatusCode, Json<RunResponse>), (StatusCode, String)> {
    let secret = state
        .settings
        .get(WEBHOOK_SECRET_KEY)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .unwrap_or_default();
    if secret.is_empty() {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "webhook secret not configured".into(),
        ));
    }
    let provided = headers
        .get("x-webhook-secret")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !constant_time_eq(provided.as_bytes(), secret.as_bytes()) {
        return Err((StatusCode::UNAUTHORIZED, "invalid webhook secret".into()));
    }

    let runtime = state.runtime.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "execution runtime not configured".into(),
    ))?;

    let agent_id = payload.agent_id.clone();
    let mut req = webhook::normalize(payload);
    // Models are agent-specific: resolve from agent_id or the payload's explicit model.
    let model = resolve_model(&state, req.model.take(), agent_id)
        .await?
        .ok_or((
            StatusCode::BAD_REQUEST,
            "agent_id or model is required".to_string(),
        ))?;
    req.model = Some(model);

    let run = start_run(
        req,
        runtime,
        &state.repo,
        &state.sessions,
        &state.transitions,
        &state.bus,
        &state.broker,
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok((StatusCode::CREATED, Json(RunResponse::from(&run))))
}

/// Constant-time byte comparison (avoids timing side channels on the secret).
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
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
