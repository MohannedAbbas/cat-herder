//! HTTP adapter (Axum): exposes the control-plane REST API (see `PLAN.md` §14).

use std::sync::Arc;

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::sse::{Event as SseEvent, KeepAlive, Sse},
    routing::{get, post, put},
};
use axum_extra::extract::cookie::CookieJar;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures::StreamExt;
use tokio::sync::RwLock;

use cat_herder_application::{
    Completion, Outcome, complete_run, decide_review, issue_budget, request_review,
    respond_approval, review_feedback_task_spec, start_run,
};
use cat_herder_credential_broker::CredentialBrokerService;
use cat_herder_credential_broker_rpc::RemoteCredentialBroker;
use cat_herder_domain::api::{
    AgentTemplate, CreateRunRequest, EventListResponse, EventResponse, RunResponse, Session,
    SessionRun, SessionSummary, repo_identity_from_url,
};
use cat_herder_domain::entities::{
    ApprovalStatus, Review, ReviewStatus, ReviewVerdict, Run, RunId, RunKind, RunState,
    RuntimeTier, Usage, WorkflowDefinition,
};
use cat_herder_domain::events::{Event, event_type};
use cat_herder_domain::policy::{AccessRequest, Decision, Policy, Role, authorize};
use cat_herder_domain::ports::{
    AgentRepository, AgentRuntime, ApprovalRepository, BudgetRepository, CheckpointRepository,
    CredentialBroker, CredentialGrantRepository, DedupRepository, EventBus, EventRepository,
    GitRepoRepository, MintedCredential, ReviewRepository, RunRepository, RunRequest,
    SessionRepository, SettingRepository, WorkflowRepository,
};
use cat_herder_runtime_agent_sandbox::SandboxRuntime;
use cat_herder_store_inmemory::{
    InMemoryEventRepository, InMemoryRunRepository, InMemoryTransitionRepository,
};

pub use auth::Auth;
pub use budget::{BudgetCounterImpl, BudgetRepo, budget_server, raise_budget_for_run};
pub use bus::EventBusImpl;
pub use crypto::AesGcmCipher;
pub use grpc::{ControlLinks, grpc_serve, grpc_server};
pub use key_store::KeySecretStore;
pub use oauth::{GITHUB, GITLAB, GitHubOAuth, GitLabOAuth, OAuthConfig};
pub use pr::{FileChange, PrClient, PrSpec, default_host_api_bases};
pub use repo::{
    AgentRepo, ApprovalRepo, AuditRepo, CheckpointRepo, CredentialGrantRepo, EventRepo,
    GitRepoRepo, ModelRepo, ProviderRepo, ReviewRepo, RunRepo, SessionRepo, SettingRepo, ToolRepo,
    TransitionRepo, WorkflowRepo,
};
pub use trigger::{
    DedupRepo, github_delivery_id, normalize_cron, normalize_github, normalize_gitlab,
    normalize_slack, verify_github_signature, verify_gitlab_token, verify_slack_signature,
    verify_webhook_secret,
};

mod admin;
mod auth;
mod budget;
mod bus;
mod crypto;
mod grpc;
mod key_store;
mod mcp;
mod oauth;
mod pr;
mod repo;
mod trigger;
mod webhook;

/// Settings key for the shared webhook secret.
pub const WEBHOOK_SECRET_KEY: &str = "webhook_secret";

/// Settings key for the supported git hosts (a JSON array of `{host,label}`).
pub const GIT_HOSTS_SETTING: &str = "git_hosts";

/// Settings key for the authorization policy (a JSON [`Policy`], §11.8).
pub const POLICY_SETTING: &str = "policy";

/// The concrete credential broker wired into the application (`PLAN.md` §11.3): either
/// the remote broker service over gRPC ([`RemoteCredentialBroker`]), or — when no broker
/// endpoint is configured — the in-process [`CredentialBrokerService`] over the local
/// grant/event repositories and the fan-out bus (single-node/dev).
#[derive(Clone)]
pub enum BrokerImpl {
    Remote(RemoteCredentialBroker),
    Local(CredentialBrokerService<CredentialGrantRepo, EventRepo, EventBusImpl>),
}

impl CredentialBroker for BrokerImpl {
    async fn mint(
        &self,
        run: RunId,
        target: &str,
        action: &str,
        scope: &str,
    ) -> Result<MintedCredential, Box<dyn std::error::Error + Send + Sync>> {
        match self {
            BrokerImpl::Remote(b) => b.mint(run, target, action, scope).await,
            BrokerImpl::Local(b) => b.mint(run, target, action, scope).await,
        }
    }

    async fn revoke(
        &self,
        run: RunId,
        grant_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        match self {
            BrokerImpl::Remote(b) => b.revoke(run, grant_id).await,
            BrokerImpl::Local(b) => b.revoke(run, grant_id).await,
        }
    }

    async fn revoke_run(&self, run: RunId) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        match self {
            BrokerImpl::Remote(b) => b.revoke_run(run).await,
            BrokerImpl::Local(b) => b.revoke_run(run).await,
        }
    }
}

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
    /// Durable result reviews (`PLAN.md` §10).
    pub reviews: ReviewRepo,
    /// The configurable workflow catalog (`PLAN.md` §7).
    pub workflows: WorkflowRepo,
    /// Minted credential-grant metadata (§11.3) — held separately from the broker so
    /// retention/deletion can purge a run's grants (§11.7).
    pub grants: CredentialGrantRepo,
    /// The JIT credential broker (`PLAN.md` §11.3). Remote (gRPC) when a broker endpoint
    /// is configured; otherwise the in-process service with an empty issuer map (whose
    /// `mint` fails gracefully and the runtime falls back to a registered repo token or
    /// none).
    pub broker: BrokerImpl,
    /// Durable per-run budget caps (`PLAN.md` §12).
    pub budgets: BudgetRepo,
    /// The atomic, real-time budget counter (Redis in production, in-memory otherwise).
    pub budget_counter: BudgetCounterImpl,
    /// Trigger dedup/replay cache (§11.5).
    pub dedup: DedupRepo,
    /// The authorization policy (§11.8): RBAC minimum roles + ABAC rules.
    pub policy: Policy,
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
        let broker = BrokerImpl::Local(CredentialBrokerService::new(
            Default::default(),
            grants.clone(),
            EventRepo::InMemory(events.clone()),
            bus.clone(),
        ));
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
            reviews: ReviewRepo::InMemory(Default::default()),
            workflows: WorkflowRepo::InMemory(Default::default()),
            grants,
            broker,
            budgets: BudgetRepo::InMemory(Default::default()),
            budget_counter: BudgetCounterImpl::InMemory(Default::default()),
            dedup: DedupRepo::InMemory(Default::default()),
            policy: Policy::default(),
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
        .route("/metrics", get(metrics))
        .route("/v1/runs", get(list_runs).post(create_run))
        .route("/v1/runs/{id}", get(get_run).delete(admin::delete_run))
        .route("/v1/runs/{id}/budget", post(raise_budget_handler))
        .route(
            "/v1/runs/{id}/events",
            get(get_run_events).post(post_run_event),
        )
        .route("/v1/runs/{id}/events/stream", get(stream_run_events))
        .route("/v1/approvals/{id}/respond", post(respond_approval_handler))
        .route("/v1/reviews", get(list_reviews))
        .route("/v1/reviews/{id}/respond", post(respond_review_handler))
        .route("/v1/runs/{id}/diff", get(get_run_diff))
        .route("/v1/runs/{id}/approval", get(get_run_approval))
        .route("/v1/runs/{id}/review", get(get_run_review))
        .route("/v1/webhooks/generic", post(webhook_generic))
        .route("/v1/webhooks/github", post(webhook_github))
        .route("/v1/webhooks/gitlab", post(webhook_gitlab))
        .route("/v1/webhooks/slack", post(webhook_slack))
        .route("/mcp", post(mcp_handler))
        .route("/v1/sessions", get(list_sessions))
        .route("/v1/sessions/{id}", get(get_session))
        .route("/v1/agents", get(list_agents))
        .route("/v1/agents/{id}", get(get_agent))
        .route("/v1/workflows", get(list_workflows))
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
        .route("/v1/admin/usage", get(admin::usage_rollup))
        .route(
            "/v1/workflows/{id}",
            put(admin::upsert_workflow).delete(admin::delete_workflow),
        )
        .with_state(state)
}

// --- Run handlers ---------------------------------------------------------

/// Expose Prometheus-format metrics (cold-start histogram, §16/§23).
async fn metrics() -> String {
    cat_herder_application::metrics::cold_start().encode_prometheus()
}

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

/// Resolve the session's role and authorize `decision` against the policy (§11.8).
/// Returns the authenticated role on success; `401` when unauthenticated, `403` when
/// the role fails the RBAC/ABAC check.
async fn require_role(
    state: &AppState,
    jar: &CookieJar,
    decision: Decision,
    attributes: std::collections::BTreeMap<String, String>,
) -> Result<Role, (StatusCode, String)> {
    let token = jar.get("session").map(|c| c.value().to_string());
    let role = match token {
        Some(t) => state.auth.session_role(&t).await,
        None => None,
    }
    .ok_or((StatusCode::UNAUTHORIZED, "authentication required".into()))?;
    let request = AccessRequest {
        decision,
        role,
        attributes,
    };
    if authorize(&state.policy, &request) {
        Ok(role)
    } else {
        Err((
            StatusCode::FORBIDDEN,
            "not authorized for this action".into(),
        ))
    }
}

/// Resolve the run's isolation tier (§13) from its selected agent template, defaulting
/// to [`RuntimeTier::Default`] for workflow runs (one sandbox runs all steps) or a
/// model-only run (no agent to derive a tier from).
pub async fn resolve_tier(
    state: &AppState,
    kind: RunKind,
    agent_id: Option<&str>,
) -> Result<RuntimeTier, (StatusCode, String)> {
    if kind == RunKind::Workflow {
        return Ok(RuntimeTier::Default);
    }
    match agent_id {
        Some(id) => state
            .agents
            .get(id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
            .map(|a| a.map(|a| a.runtime_tier).unwrap_or_default()),
        None => Ok(RuntimeTier::Default),
    }
}

/// Accept a task, orchestrate it through the lifecycle, and spawn a Sandbox.
async fn create_run(
    State(state): State<AppState>,
    jar: CookieJar,
    Json(req): Json<CreateRunRequest>,
) -> Result<(StatusCode, Json<RunResponse>), (StatusCode, String)> {
    // §11.8: run creation (and its clone-credential mint) are authorization decisions.
    require_role(&state, &jar, Decision::RunCreate, Default::default()).await?;
    if let Some(repo) = req.repo.as_deref()
        && let Some(identity) = repo_identity_from_url(repo)
    {
        require_role(
            &state,
            &jar,
            Decision::CredentialMint,
            std::collections::BTreeMap::from([
                ("target".into(), identity.id()),
                ("action".into(), "clone".into()),
                ("scope".into(), "read".into()),
            ]),
        )
        .await?;
    }
    let runtime = state.runtime.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "execution runtime not configured".into(),
    ))?;

    // The isolation tier (§13) comes from the selected agent template; a workflow run
    // (one sandbox running all steps) and a model-only run default to the default tier.
    let runtime_tier = resolve_tier(&state, req.kind, req.agent_id.as_deref()).await?;

    // Models are agent-specific; a run needs a resolvable model. A workflow run's
    // per-step models come from its definition (resolved at spawn), so a single
    // `model` is not required — but the referenced workflow must exist.
    let model = if req.kind == RunKind::Workflow {
        let wref = req.workflow_ref.as_deref().ok_or((
            StatusCode::BAD_REQUEST,
            "workflow_ref is required for a workflow run".to_string(),
        ))?;
        state
            .workflows
            .get(wref)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .ok_or((
                StatusCode::BAD_REQUEST,
                format!("workflow '{wref}' not found"),
            ))?;
        None
    } else {
        Some(
            resolve_model(&state, req.model, req.agent_id)
                .await?
                .ok_or((
                    StatusCode::BAD_REQUEST,
                    "agent_id or model is required".to_string(),
                ))?,
        )
    };

    let run_req = RunRequest {
        kind: req.kind,
        task_spec: req.task_spec,
        dedup_key: String::new(),
        max_concurrency: req.max_concurrency,
        trigger: "api".into(),
        actor: req.actor,
        model,
        repo: req.repo,
        language: req.language,
        workflow_ref: req.workflow_ref,
        runtime_tier,
        budget: req.budget,
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

    // Budget lease (§12): persist the cap + zero the counter so the proxy enforces it
    // from the first call. Best-effort — a lease failure must not fail the run.
    if let Err(e) = issue_budget(&state.budgets, &state.budget_counter, run.id, req.budget).await {
        eprintln!("budget lease failed for run {}: {e}", run.id.0);
    }

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

/// Request body for `POST /v1/runs/{id}/budget` (`PLAN.md` §12) — raise (or set) a
/// run's budget cap; a `BudgetPaused` run resumes to `Running`.
#[derive(Debug, serde::Deserialize)]
pub struct RaiseBudgetRequest {
    pub cap: u64,
}

async fn raise_budget_handler(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(id): Path<u64>,
    Json(req): Json<RaiseBudgetRequest>,
) -> Result<Json<RunResponse>, (StatusCode, String)> {
    // Budget management is a run-management decision (§12); gate it like run creation.
    require_role(&state, &jar, Decision::RunCreate, Default::default()).await?;

    let run_id = RunId(id);
    raise_budget_for_run(&state, run_id, req.cap)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let run = state
        .repo
        .get(run_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "run not found".to_string()))?;
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
    jar: CookieJar,
    Path(id): Path<String>,
    Json(req): Json<RespondApprovalRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // §11.8: approval is an authorization decision ("who may approve what").
    require_role(&state, &jar, Decision::Approval, Default::default()).await?;

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

/// Request body for `POST /v1/reviews/{id}/respond` (`PLAN.md` §10). `verdict` is
/// `approved` | `rejected` | `changes_requested`; `feedback` carries the reviewer's
/// comments (required for change requests).
#[derive(Debug, serde::Deserialize)]
pub struct RespondReviewRequest {
    pub verdict: ReviewVerdict,
    #[serde(default)]
    pub feedback: Option<String>,
}

/// Millis-since-epoch for a [`std::time::SystemTime`] (wire form for review timing).
fn to_millis(t: std::time::SystemTime) -> u64 {
    t.duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// List the result-review queue (`PLAN.md` §10): pending reviews, oldest first,
/// enriched with each run's task + report + PR link so a reviewer can see what they
/// are reviewing before responding.
async fn list_reviews(
    State(state): State<AppState>,
    jar: CookieJar,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, String)> {
    require_role(&state, &jar, Decision::ResultReview, Default::default()).await?;

    let reviews = state
        .reviews
        .list_pending()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut out = Vec::with_capacity(reviews.len());
    for review in reviews {
        let run = state.repo.get(review.run_id).await.ok().flatten();
        out.push(serde_json::json!({
            "id": review.id,
            "run_id": review.run_id.0,
            "status": review.status,
            "verdict": review.verdict,
            "reviewer": review.reviewer,
            "feedback": review.feedback,
            "created_at": to_millis(review.created_at),
            "reviewed_at": review.reviewed_at.map(to_millis),
            "deadline_at": to_millis(review.deadline_at),
            "task_spec": run.as_ref().map(|r| r.task_spec.clone()),
            "report": run.as_ref().and_then(|r| r.report.clone()),
            "pr_url": run.as_ref().and_then(|r| r.pr_url.clone()),
        }));
    }
    Ok(Json(out))
}

/// Record a reviewer's verdict on a result review (§10). `approved` moves the run to
/// `Reviewed`; `rejected`/`changes_requested` move it to `Rejected` and spawn a
/// follow-up run carrying the feedback as its task spec.
async fn respond_review_handler(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(id): Path<String>,
    Json(req): Json<RespondReviewRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // §11.8: result review is an authorization decision ("who may mark a result
    // reviewed/approved").
    require_role(&state, &jar, Decision::ResultReview, Default::default()).await?;

    // 404 for unknown, 409 for an already-decided review.
    let existing = state
        .reviews
        .get(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "review not found".to_string()))?;
    if existing.status != ReviewStatus::Pending {
        return Err((StatusCode::CONFLICT, "review already decided".to_string()));
    }

    // The reviewer's identity is the session's username (password admin or SSO login).
    let token = jar.get("session").map(|c| c.value().to_string());
    let reviewer = match token {
        Some(t) => state.auth.session_username(&t).await,
        None => None,
    };

    let review = decide_review(
        &id,
        req.verdict,
        reviewer,
        req.feedback.clone(),
        &state.reviews,
        &state.events,
        &state.repo,
        &state.sessions,
        &state.transitions,
        &state.bus,
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Rejection / change requests feed back as a new run (review-feedback task_spec).
    if matches!(
        review.verdict,
        Some(ReviewVerdict::Rejected | ReviewVerdict::ChangesRequested)
    ) {
        spawn_review_followup(&state, &review).await;
    }

    Ok(Json(serde_json::json!({
        "id": review.id,
        "run_id": review.run_id.0,
        "status": review.status,
        "verdict": review.verdict,
        "reviewer": review.reviewer,
        "feedback": review.feedback,
    })))
}

/// Spawn a follow-up run carrying the reviewer's feedback as its task spec (§10).
/// Reuses the reviewed run's repo/model/language/etc. so the new run re-does the work
/// with the requested changes. Best-effort: with no runtime (or a missing parent) the
/// review still stands, just without a follow-up run.
async fn spawn_review_followup(state: &AppState, review: &Review) {
    let Some(runtime) = &state.runtime else {
        return;
    };
    let Ok(Some(parent)) = state.repo.get(review.run_id).await else {
        return;
    };
    let task_spec = review_feedback_task_spec(
        &parent.task_spec,
        parent.id,
        review.verdict.unwrap_or(ReviewVerdict::ChangesRequested),
        review.reviewer.as_deref(),
        review.feedback.as_deref(),
    );
    let req = RunRequest {
        kind: parent.kind,
        task_spec,
        dedup_key: String::new(),
        max_concurrency: parent.max_concurrency,
        trigger: "review".into(),
        actor: parent.actor.clone(),
        model: parent.model.clone(),
        repo: parent.repo.clone(),
        language: parent.language.clone(),
        workflow_ref: None,
        runtime_tier: parent.runtime_tier,
        budget: None,
    };
    match start_run(
        req,
        runtime,
        &state.repo,
        &state.sessions,
        &state.transitions,
        &state.bus,
        &state.broker,
    )
    .await
    {
        Ok(run) => eprintln!(
            "spawned review follow-up run {} for review {}",
            run.id.0, review.id
        ),
        Err(e) => eprintln!("review follow-up run failed for review {}: {e}", review.id),
    }
}

/// Decode a base64 file content blob to UTF-8 text for the diff view (best-effort:
/// binary files yield `None`).
fn decode_base64_text(b64: &str) -> Option<String> {
    let bytes = STANDARD.decode(b64).ok()?;
    String::from_utf8(bytes).ok()
}

/// The workspace diff for a run — the changed files the harness reported on
/// `finished` (stored in the `agent.finished` event), with base64 content decoded to
/// text for display. Empty `changes` when the run produced no reviewable output.
async fn get_run_diff(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let run_id = RunId(id);
    state
        .repo
        .get(run_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "run not found".to_string()))?;

    // The diff is the `changes` payload of the latest `agent.finished` event.
    let events = state
        .events
        .list(run_id, 0, 1000)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let changes = events
        .iter()
        .rev()
        .find(|e| e.event_type == event_type::AGENT_FINISHED)
        .and_then(|e| e.payload.get("changes").cloned())
        .and_then(|v| serde_json::from_value::<Vec<FileChange>>(v).ok())
        .unwrap_or_default();

    let view: Vec<serde_json::Value> = changes
        .iter()
        .map(|fc| {
            let content = fc
                .content_base64
                .as_deref()
                .and_then(decode_base64_text);
            serde_json::json!({
                "path": fc.path,
                "status": fc.status,
                "content": content,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({ "changes": view })))
}

/// The pending approval for a run (the interrupt checkpoint + pending tool call), or
/// `404` when the run has none. This is the "de-blind" view an approver reads from the
/// deep link before responding.
async fn get_run_approval(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let run_id = RunId(id);
    state
        .repo
        .get(run_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "run not found".to_string()))?;

    let approval = state
        .approvals
        .list_pending()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .into_iter()
        .find(|a| a.run_id == run_id)
        .ok_or((StatusCode::NOT_FOUND, "no pending approval".to_string()))?;

    Ok(Json(serde_json::json!({
        "id": approval.id,
        "run_id": approval.run_id.0,
        "status": approval.status,
        "resume_token": approval.resume_token,
        "payload": approval.payload,
        "created_at": to_millis(approval.created_at),
    })))
}

/// The pending review for a run (verdict options + deadline + feedback), or `404` when
/// the run has none. This is the "de-blind" view a reviewer reads before responding.
async fn get_run_review(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let run_id = RunId(id);
    state
        .repo
        .get(run_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "run not found".to_string()))?;

    let review = state
        .reviews
        .list_pending()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .into_iter()
        .find(|r| r.run_id == run_id)
        .ok_or((StatusCode::NOT_FOUND, "no pending review".to_string()))?;

    Ok(Json(serde_json::json!({
        "id": review.id,
        "run_id": review.run_id.0,
        "status": review.status,
        "verdict": review.verdict,
        "reviewer": review.reviewer,
        "feedback": review.feedback,
        "created_at": to_millis(review.created_at),
        "deadline_at": to_millis(review.deadline_at),
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
                let has_changes = changes.as_ref().is_some_and(|c| !c.is_empty());
                maybe_open_pr(state, &run, report, changes).await;

                // A run that produced reviewable changes (a diff/PR) enters the result
                // review loop (§10): Succeeded -> AwaitingReview.
                if has_changes {
                    request_review_for_run(state, &run).await;
                }
            }

            // Re-fetch so the response reflects the review transition applied above
            // (Succeeded -> AwaitingReview); the PR link still lands async in the
            // background. A failure here leaves the already-recorded run unchanged.
            let run = state
                .repo
                .get(run_id)
                .await
                .ok()
                .flatten()
                .unwrap_or(run);

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

/// Enter a `Succeeded` run that produced reviewable changes into the result-review
/// loop (§10). The deadline is configurable via `CAT_HERDER_REVIEW_DEADLINE_SECS`
/// (default 72h). Best-effort — a failure here must not fail the already-recorded run.
async fn request_review_for_run(state: &AppState, run: &Run) {
    let deadline_secs = env_u32_or("CAT_HERDER_REVIEW_DEADLINE_SECS", 72 * 60 * 60) as u64;
    let deadline_at =
        std::time::SystemTime::now() + std::time::Duration::from_secs(deadline_secs);
    if let Err(e) = request_review(
        run.id,
        deadline_at,
        &state.reviews,
        &state.events,
        &state.repo,
        &state.sessions,
        &state.transitions,
        &state.bus,
    )
    .await
    {
        eprintln!("review request failed for run {}: {e}", run.id.0);
    }
}

/// Hard-delete a run and all its data (§11.7): revoke still-active credentials, then
/// purge the run's events, checkpoints, approvals, reviews, budgets, grants, session
/// display row, and the run itself. Postgres cascades most of this via the `runs` FK,
/// but the explicit deletes keep the in-memory store correct too. A failed credential
/// revocation is logged, not fatal.
pub(crate) async fn purge_run(state: &AppState, run_id: RunId) -> Result<(), String> {
    if let Err(e) = state.broker.revoke_run(run_id).await {
        eprintln!("credential revocation failed for run {}: {e}", run_id.0);
    }

    state
        .events
        .delete_run(run_id)
        .await
        .map_err(|e| e.to_string())?;
    state
        .checkpoints
        .delete_run(run_id)
        .await
        .map_err(|e| e.to_string())?;
    state
        .approvals
        .delete_run(run_id)
        .await
        .map_err(|e| e.to_string())?;
    state
        .reviews
        .delete_run(run_id)
        .await
        .map_err(|e| e.to_string())?;
    state
        .budgets
        .delete(run_id)
        .await
        .map_err(|e| e.to_string())?;
    state
        .grants
        .delete_run(run_id)
        .await
        .map_err(|e| e.to_string())?;
    state
        .sessions
        .delete_run(run_id)
        .await
        .map_err(|e| e.to_string())?;
    // The run itself, last (the Postgres FK cascade is a backstop).
    state.repo.delete(run_id).await.map_err(|e| e.to_string())?;
    Ok(())
}

/// Retention sweep (§11.7): hard-deletes terminal runs whose `finished_at` (or
/// `created_at`) is older than `retention_days`. Returns the number purged. A
/// `retention_days` of `0` disables the sweep.
pub async fn retention_sweep(state: &AppState, retention_days: u64) -> usize {
    if retention_days == 0 {
        return 0;
    }
    let cutoff = std::time::SystemTime::now()
        - std::time::Duration::from_secs(retention_days.saturating_mul(86_400));
    let runs = match state.repo.list_all().await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("retention sweep: list failed: {e}");
            return 0;
        }
    };
    let mut purged = 0;
    for run in runs {
        // Purge only "done" runs: terminal states, or `Succeeded` (a finished leaf with
        // no reviewable output). Runs still awaiting input/review are left alone.
        let done = cat_herder_domain::state::is_terminal(run.state)
            || run.state == RunState::Succeeded;
        if !done {
            continue;
        }
        let when = run.finished_at.or(Some(run.created_at));
        if !when.is_some_and(|t| t < cutoff) {
            continue;
        }
        match purge_run(state, run.id).await {
            Ok(()) => {
                purged += 1;
                eprintln!("retention: purged run {}", run.id.0);
            }
            Err(e) => eprintln!("retention: purge of run {} failed: {e}", run.id.0),
        }
    }
    purged
}

/// Parse a `u32` env var, falling back to `default` when unset/blank/invalid.
fn env_u32_or(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(default)
}

/// Admission control for triggers (`PLAN.md` §11.5): dedup/replay cache, per-source
/// cooldown, and a global concurrency cap. Returns a denial or `Ok(())` to admit.
async fn admit_trigger(
    state: &AppState,
    source: &str,
    dedup_key: &str,
) -> Result<(), (StatusCode, String)> {
    use cat_herder_domain::state::is_terminal;

    // Dedup/replay: a replayed event maps to the one already-accepted run.
    if !dedup_key.is_empty() {
        let window =
            std::time::Duration::from_secs(env_u32_or("CAT_HERDER_DEDUP_WINDOW_SECS", 3600) as u64);
        let duplicate = state
            .dedup
            .check_and_record(&format!("dedup:{dedup_key}"), window)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        if duplicate {
            return Err((StatusCode::CONFLICT, "duplicate event".into()));
        }
    }

    // Per-source cooldown.
    let cooldown = env_u32_or("CAT_HERDER_SOURCE_COOLDOWN_SECS", 0);
    if cooldown > 0 {
        let cooling = state
            .dedup
            .check_and_record(
                &format!("cooldown:{source}"),
                std::time::Duration::from_secs(cooldown as u64),
            )
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        if cooling {
            return Err((StatusCode::TOO_MANY_REQUESTS, "source cooling down".into()));
        }
    }

    // Global concurrency cap on spawned runs.
    let cap = env_u32_or("CAT_HERDER_MAX_CONCURRENT_RUNS", 0);
    if cap > 0 {
        let runs = state
            .repo
            .list_all()
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let active = runs.iter().filter(|r| !is_terminal(r.state)).count();
        if active >= cap as usize {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "concurrency cap reached".into(),
            ));
        }
    }

    Ok(())
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

    let agent_id = payload.agent_id.clone();
    let mut req = webhook::normalize(payload);

    // Admission control (§11.5): dedup/replay, per-source cooldown, concurrency cap.
    // Checked before the runtime pre-flight so a replayed event is rejected even when
    // the system is mid-deploy.
    admit_trigger(&state, "webhook", &req.dedup_key).await?;

    let runtime = state.runtime.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "execution runtime not configured".into(),
    ))?;

    // Models are agent-specific: resolve from agent_id or the payload's explicit model.
    let model = resolve_model(&state, req.model.take(), agent_id)
        .await?
        .ok_or((
            StatusCode::BAD_REQUEST,
            "agent_id or model is required".to_string(),
        ))?;
    req.model = Some(model);

    let budget = req.budget;
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

    if let Err(e) = issue_budget(&state.budgets, &state.budget_counter, run.id, budget).await {
        eprintln!("budget lease failed for run {}: {e}", run.id.0);
    }

    Ok((StatusCode::CREATED, Json(RunResponse::from(&run))))
}

/// Query params for source-specific webhooks: an explicit `model` (or `agent_id`)
/// selects the run's model, since source payloads don't carry one.
#[derive(Debug, serde::Deserialize)]
struct SourceQuery {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    agent_id: Option<String>,
}

/// A required secret from the environment (or `503` when unset).
fn require_env_secret(name: &str) -> Result<String, (StatusCode, String)> {
    let value = std::env::var(name).ok().filter(|s| !s.trim().is_empty());
    value.ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        format!("{name} not configured"),
    ))
}

/// Common source-webhook tail: resolve the model, admit (dedup/cooldown/cap), and run.
async fn execute_source_run(
    state: &AppState,
    source: &str,
    dedup_key: &str,
    mut req: RunRequest,
    query: SourceQuery,
) -> Result<(StatusCode, Json<RunResponse>), (StatusCode, String)> {
    let model = resolve_model(state, query.model, query.agent_id)
        .await?
        .ok_or((
            StatusCode::BAD_REQUEST,
            "model or agent_id is required".to_string(),
        ))?;
    req.model = Some(model);

    admit_trigger(state, source, dedup_key).await?;

    let runtime = state.runtime.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "execution runtime not configured".into(),
    ))?;
    let budget = req.budget;
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

    if let Err(e) = issue_budget(&state.budgets, &state.budget_counter, run.id, budget).await {
        eprintln!("budget lease failed for run {}: {e}", run.id.0);
    }

    Ok((StatusCode::CREATED, Json(RunResponse::from(&run))))
}

/// GitHub App webhook: verify `X-Hub-Signature-256`, normalize, dedup by delivery id.
async fn webhook_github(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<SourceQuery>,
    body: Bytes,
) -> Result<(StatusCode, Json<RunResponse>), (StatusCode, String)> {
    let secret = require_env_secret("CAT_HERDER_GITHUB_WEBHOOK_SECRET")?;
    let sig = headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !verify_github_signature(&secret, &body, sig) {
        return Err((StatusCode::UNAUTHORIZED, "invalid signature".into()));
    }
    let event = headers
        .get("x-github-event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let payload: serde_json::Value =
        serde_json::from_slice(&body).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let mut req = normalize_github(event, &payload);
    req.dedup_key = github_delivery_id(
        headers
            .get("x-github-delivery")
            .and_then(|v| v.to_str().ok()),
    );
    execute_source_run(&state, "github", &req.dedup_key.clone(), req, query).await
}

/// GitLab webhook: verify `X-Gitlab-Token`, normalize.
async fn webhook_gitlab(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<SourceQuery>,
    body: Bytes,
) -> Result<(StatusCode, Json<RunResponse>), (StatusCode, String)> {
    let secret = require_env_secret("CAT_HERDER_GITLAB_WEBHOOK_TOKEN")?;
    let token = headers
        .get("x-gitlab-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !verify_gitlab_token(&secret, token) {
        return Err((StatusCode::UNAUTHORIZED, "invalid token".into()));
    }
    let payload: serde_json::Value =
        serde_json::from_slice(&body).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let mut req = normalize_gitlab(&payload);
    req.dedup_key = headers
        .get("x-gitlab-event-uuid")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    execute_source_run(&state, "gitlab", &req.dedup_key.clone(), req, query).await
}

/// Slack slash command: verify `X-Slack-Signature`, normalize the free-text command.
async fn webhook_slack(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<SourceQuery>,
    body: Bytes,
) -> Result<(StatusCode, Json<RunResponse>), (StatusCode, String)> {
    let secret = require_env_secret("CAT_HERDER_SLACK_SIGNING_SECRET")?;
    let timestamp = headers
        .get("x-slack-request-timestamp")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let signature = headers
        .get("x-slack-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !verify_slack_signature(&secret, &body, timestamp, signature) {
        return Err((StatusCode::UNAUTHORIZED, "invalid signature".into()));
    }
    // Slack form-encoded: `text=...`
    let text = String::from_utf8_lossy(&body);
    let command = form_field(&text, "text").unwrap_or_default();
    let mut req = normalize_slack(&command);
    req.dedup_key = format!("slack:{timestamp}");
    execute_source_run(&state, "slack", &req.dedup_key.clone(), req, query).await
}

/// Extract a form field (`key=value` pairs, `application/x-www-form-urlencoded`).
fn form_field(body: &str, key: &str) -> Option<String> {
    for pair in body.split('&') {
        let (k, v) = pair.split_once('=')?;
        if k == key {
            return Some(url_decode(v).unwrap_or_else(|| v.to_string()));
        }
    }
    None
}

/// Minimal `application/x-www-form-urlencoded` percent-decoding (`+` → space).
fn url_decode(s: &str) -> Option<String> {
    let bytes = s.replace('+', " ");
    let mut out = Vec::with_capacity(bytes.len());
    let chars: Vec<char> = bytes.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '%' && i + 2 < chars.len() {
            let hex = chars[i + 1..i + 3].iter().collect::<String>();
            let b = u8::from_str_radix(&hex, 16).ok()?;
            out.push(b);
            i += 3;
        } else {
            out.push(chars[i] as u8);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
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

/// The configurable workflow catalog (`PLAN.md` §7) — public read, like the agent
/// catalog. Write paths (`PUT`/`DELETE`) are admin-gated in `admin.rs`.
async fn list_workflows(
    State(state): State<AppState>,
) -> Json<Vec<WorkflowDefinition>> {
    Json(state.workflows.list().await.unwrap_or_default())
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
