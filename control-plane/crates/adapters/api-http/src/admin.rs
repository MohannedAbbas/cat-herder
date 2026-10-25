//! Admin + auth HTTP handlers (see `PLAN.md` §3/§14). Everything under `/v1/admin`
//! requires an authenticated admin session (HttpOnly cookie).

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use axum_extra::extract::cookie::{Cookie, CookieJar};
use serde::Deserialize;

use cat_herder_domain::api::{
    AgentTemplate, AuditEntry, GitHost, GitRepo, Model, Provider, Tool, default_git_hosts,
    new_webhook_secret, repo_identity,
};
use cat_herder_domain::entities::GitRepo as GitRepoEntity;
use cat_herder_domain::ports::{
    AgentRepository, AuditRepository, GitRepoRepository, ModelRepository, ProviderRepository,
    SettingRepository, ToolRepository,
};

use super::AppState;

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

async fn require_admin(state: &AppState, jar: &CookieJar) -> Result<String, (StatusCode, String)> {
    let token = jar.get("session").map(|c| c.value().to_string());
    match token {
        Some(t) if state.auth.valid_session(&t).await => {
            Ok(state.auth.admin_username().to_string())
        }
        _ => Err((StatusCode::UNAUTHORIZED, "authentication required".into())),
    }
}

/// Record an admin action (best-effort — audit failure never blocks the action).
async fn audit(state: &AppState, actor: &str, action: &str, resource: &str, id: &str) {
    let _ = state.audit.record(actor, action, resource, id).await;
}

// --- auth ------------------------------------------------------------------

pub async fn login(
    State(state): State<AppState>,
    jar: CookieJar,
    Json(creds): Json<LoginRequest>,
) -> Result<(StatusCode, CookieJar, Json<serde_json::Value>), (StatusCode, String)> {
    match state.auth.login(&creds.username, &creds.password).await {
        Some(token) => {
            Ok((
                StatusCode::OK,
                jar.add(session_cookie(token)),
                Json(serde_json::json!({ "ok": true })),
            ))
        }
        None => Err((StatusCode::UNAUTHORIZED, "invalid credentials".into())),
    }
}

pub async fn logout(
    State(state): State<AppState>,
    jar: CookieJar,
) -> (StatusCode, CookieJar) {
    if let Some(c) = jar.get("session") {
        state.auth.logout(c.value()).await;
    }
    let removal = Cookie::build(("session", "")).path("/").build();
    (StatusCode::OK, jar.remove(removal))
}

pub async fn me(State(state): State<AppState>, jar: CookieJar) -> Json<serde_json::Value> {
    let token = jar.get("session").map(|c| c.value().to_string());
    let username = match token {
        Some(t) => state.auth.session_username(&t).await,
        None => None,
    };
    let authenticated = username.is_some();
    Json(serde_json::json!({
        "authenticated": authenticated,
        "username": username.map(serde_json::Value::String).unwrap_or(serde_json::Value::Null),
    }))
}

/// Report which SSO providers are configured so the UI knows which buttons to
/// render. Public (login page is unauthenticated); leaks only provider names.
pub async fn sso_providers(State(state): State<AppState>) -> Json<serde_json::Value> {
    let mut providers = Vec::with_capacity(2);
    if state.mock_oauth.is_some() {
        // Dev-only mock mode pretends both providers are configured.
        providers.push("github");
        providers.push("gitlab");
    } else {
        if state.github_oauth.is_some() {
            providers.push("github");
        }
        if state.gitlab_oauth.is_some() {
            providers.push("gitlab");
        }
    }
    Json(serde_json::json!({ "providers": providers }))
}

/// Dev-only mock OAuth: issue an admin session directly for the mock username and
/// redirect to `/` (mirrors a successful callback) without contacting the provider.
async fn mock_login(state: &AppState, jar: CookieJar, username: &str) -> Response {
    let token = state.auth.create_session_for(username.to_string()).await;
    let response = Redirect::to("/").into_response();
    (jar.add(session_cookie(token)), response).into_response()
}

// --- GitHub OAuth SSO ------------------------------------------------------

/// Query params on the OAuth callback (`?code=...&state=...`).
#[derive(Debug, Deserialize)]
pub struct GitHubCallback {
    pub code: String,
    /// CSRF token that `github_login` stashed in a short-lived cookie.
    #[serde(default)]
    pub state: Option<String>,
}

fn session_cookie(token: String) -> Cookie<'static> {
    Cookie::build(("session", token))
        .http_only(true)
        .same_site(axum_extra::extract::cookie::SameSite::Lax)
        .path("/")
        .build()
}

/// Begin the GitHub OAuth flow: set a CSRF `state` cookie and redirect to GitHub.
pub async fn github_login(State(state): State<AppState>, jar: CookieJar) -> Response {
    if let Some(mock_user) = state.mock_oauth.clone() {
        return mock_login(&state, jar, &mock_user).await;
    }
    match state.github_oauth.as_ref() {
        Some(oauth) => {
            let csrf = uuid::Uuid::new_v4().to_string();
            let cookie = Cookie::build(("gh_oauth_state", csrf.clone()))
                .http_only(true)
                .same_site(axum_extra::extract::cookie::SameSite::Lax)
                .path("/")
                .build();
            // Append the CSRF state to GitHub's authorize URL.
            let url = format!("{}&state={}", oauth.config().authorize_url(), csrf);
            (jar.add(cookie), Redirect::temporary(url.as_str())).into_response()
        }
        None => (StatusCode::NOT_FOUND, "GitHub OAuth not configured").into_response(),
    }
}

/// Complete the GitHub OAuth flow: verify `state`, exchange the code, fetch the
/// user, enforce the allow-list, then issue a normal admin session.
pub async fn github_callback(
    State(state): State<AppState>,
    jar: CookieJar,
    axum::extract::Query(params): axum::extract::Query<GitHubCallback>,
) -> Response {
    let Some(oauth) = state.github_oauth.as_ref() else {
        return (StatusCode::NOT_FOUND, "GitHub OAuth not configured").into_response();
    };

    // CSRF check: the callback state must match the cookie set at login.
    let expected = jar.get("gh_oauth_state").map(|c| c.value().to_string());
    if expected.is_none() || expected.as_deref() != params.state.as_deref() {
        return (StatusCode::BAD_REQUEST, "invalid OAuth state").into_response();
    }
    let clear_state = Cookie::build(("gh_oauth_state", "")).path("/").build();

    match oauth.authenticate(&params.code).await {
        Ok(login) => {
            let token = state.auth.create_session_for(login).await;
            let response = Redirect::to("/").into_response();
            (jar.remove(clear_state).add(session_cookie(token)), response).into_response()
        }
        Err(super::oauth::AuthOutcome::NotAllowed) => {
            (StatusCode::FORBIDDEN, "GitHub user not authorized").into_response()
        }
        Err(_) => (StatusCode::BAD_GATEWAY, "GitHub authentication failed").into_response(),
    }
}

// --- GitLab OAuth SSO ------------------------------------------------------

/// Query params on the GitLab OAuth callback (`?code=...&state=...`).
#[derive(Debug, Deserialize)]
pub struct GitLabCallback {
    pub code: String,
    /// CSRF token that `gitlab_login` stashed in a short-lived cookie.
    #[serde(default)]
    pub state: Option<String>,
}

/// Begin the GitLab OAuth flow: set a CSRF `state` cookie and redirect to GitLab.
pub async fn gitlab_login(State(state): State<AppState>, jar: CookieJar) -> Response {
    if let Some(mock_user) = state.mock_oauth.clone() {
        return mock_login(&state, jar, &mock_user).await;
    }
    match state.gitlab_oauth.as_ref() {
        Some(oauth) => {
            let csrf = uuid::Uuid::new_v4().to_string();
            let cookie = Cookie::build((super::oauth::GITLAB.state_cookie, csrf.clone()))
                .http_only(true)
                .same_site(axum_extra::extract::cookie::SameSite::Lax)
                .path("/")
                .build();
            // Append the CSRF state to GitLab's authorize URL.
            let url = format!("{}&state={}", oauth.config().authorize_url(), csrf);
            (jar.add(cookie), Redirect::temporary(url.as_str())).into_response()
        }
        None => (StatusCode::NOT_FOUND, "GitLab OAuth not configured").into_response(),
    }
}

/// Complete the GitLab OAuth flow: verify `state`, exchange the code, fetch the
/// user, enforce the allow-list, then issue a normal admin session.
pub async fn gitlab_callback(
    State(state): State<AppState>,
    jar: CookieJar,
    axum::extract::Query(params): axum::extract::Query<GitLabCallback>,
) -> Response {
    let Some(oauth) = state.gitlab_oauth.as_ref() else {
        return (StatusCode::NOT_FOUND, "GitLab OAuth not configured").into_response();
    };

    // CSRF check: the callback state must match the cookie set at login.
    let expected = jar
        .get(super::oauth::GITLAB.state_cookie)
        .map(|c| c.value().to_string());
    if expected.is_none() || expected.as_deref() != params.state.as_deref() {
        return (StatusCode::BAD_REQUEST, "invalid OAuth state").into_response();
    }
    let clear_state = Cookie::build((super::oauth::GITLAB.state_cookie, ""))
        .path("/")
        .build();

    match oauth.authenticate(&params.code).await {
        Ok(login) => {
            let token = state.auth.create_session_for(login).await;
            let response = Redirect::to("/").into_response();
            (jar.remove(clear_state).add(session_cookie(token)), response).into_response()
        }
        Err(super::oauth::AuthOutcome::NotAllowed) => {
            (StatusCode::FORBIDDEN, "GitLab user not authorized").into_response()
        }
        Err(_) => (StatusCode::BAD_GATEWAY, "GitLab authentication failed").into_response(),
    }
}

// --- agents ----------------------------------------------------------------

pub async fn upsert_agent(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(id): Path<String>,
    Json(mut agent): Json<AgentTemplate>,
) -> Result<Json<AgentTemplate>, (StatusCode, String)> {
    let actor = require_admin(&state, &jar).await?;
    agent.id = id;
    let saved = state
        .agents
        .save(&agent)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    audit(&state, &actor, "agent.upsert", "agent", &saved.id).await;
    Ok(Json(saved))
}

pub async fn delete_agent(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let actor = require_admin(&state, &jar).await?;
    state
        .agents
        .delete(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    audit(&state, &actor, "agent.delete", "agent", &id).await;
    Ok(StatusCode::NO_CONTENT)
}

// --- providers -------------------------------------------------------------

pub async fn list_providers(
    State(state): State<AppState>,
    jar: CookieJar,
) -> Result<Json<Vec<Provider>>, (StatusCode, String)> {
    require_admin(&state, &jar).await?;
    state
        .providers
        .list()
        .await
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

pub async fn upsert_provider(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(id): Path<String>,
    Json(mut provider): Json<Provider>,
) -> Result<Json<Provider>, (StatusCode, String)> {
    let actor = require_admin(&state, &jar).await?;
    provider.id = id;
    let saved = state
        .providers
        .save(&provider)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    audit(&state, &actor, "provider.upsert", "provider", &saved.id).await;
    Ok(Json(saved))
}

pub async fn delete_provider(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let actor = require_admin(&state, &jar).await?;
    state
        .providers
        .delete(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    audit(&state, &actor, "provider.delete", "provider", &id).await;
    Ok(StatusCode::NO_CONTENT)
}

// --- models ----------------------------------------------------------------

pub async fn list_models(
    State(state): State<AppState>,
    jar: CookieJar,
) -> Result<Json<Vec<Model>>, (StatusCode, String)> {
    require_admin(&state, &jar).await?;
    state
        .models
        .list()
        .await
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

pub async fn upsert_model(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(id): Path<String>,
    Json(mut model): Json<Model>,
) -> Result<Json<Model>, (StatusCode, String)> {
    let actor = require_admin(&state, &jar).await?;
    model.id = id;
    let saved = state
        .models
        .save(&model)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    audit(&state, &actor, "model.upsert", "model", &saved.id).await;
    Ok(Json(saved))
}

pub async fn delete_model(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let actor = require_admin(&state, &jar).await?;
    state
        .models
        .delete(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    audit(&state, &actor, "model.delete", "model", &id).await;
    Ok(StatusCode::NO_CONTENT)
}

// --- audit -----------------------------------------------------------------

pub async fn list_audit(
    State(state): State<AppState>,
    jar: CookieJar,
) -> Result<Json<Vec<AuditEntry>>, (StatusCode, String)> {
    require_admin(&state, &jar).await?;
    state
        .audit
        .list(100)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

// --- tools -----------------------------------------------------------------

pub async fn list_tools(
    State(state): State<AppState>,
    jar: CookieJar,
) -> Result<Json<Vec<Tool>>, (StatusCode, String)> {
    require_admin(&state, &jar).await?;
    state
        .tools
        .list()
        .await
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

pub async fn upsert_tool(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(id): Path<String>,
    Json(mut tool): Json<Tool>,
) -> Result<Json<Tool>, (StatusCode, String)> {
    let actor = require_admin(&state, &jar).await?;
    tool.id = id;
    let saved = state
        .tools
        .save(&tool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    audit(&state, &actor, "tool.upsert", "tool", &saved.id).await;
    Ok(Json(saved))
}

pub async fn delete_tool(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let actor = require_admin(&state, &jar).await?;
    state
        .tools
        .delete(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    audit(&state, &actor, "tool.delete", "tool", &id).await;
    Ok(StatusCode::NO_CONTENT)
}

// --- git repos -------------------------------------------------------------

/// Write model for `PUT /v1/admin/repos`. Only the host + repo path are required; the
/// canonical `id`/`name`/`url` are derived server-side. Both tokens are write-only.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitRepoWrite {
    /// Git host (e.g. `github.com`), from the configured git-hosts list.
    pub host: String,
    /// Repo path, e.g. `org/repo` or `group/subgroup/repo`.
    pub repo: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Read access token (for cloning). Blank/omitted on update keeps the existing token.
    #[serde(default)]
    pub token: Option<String>,
    /// Write access token (for opening PRs/MRs). Blank/omitted on update keeps the existing token.
    #[serde(default)]
    pub write_token: Option<String>,
}

/// Query model for `DELETE /v1/admin/repos?id=HOST/PATH`.
#[derive(Debug, Deserialize)]
pub struct DeleteRepoQuery {
    pub id: String,
}

fn default_true() -> bool {
    true
}

pub async fn list_repos(
    State(state): State<AppState>,
    jar: CookieJar,
) -> Result<Json<Vec<GitRepo>>, (StatusCode, String)> {
    require_admin(&state, &jar).await?;
    let repos = state
        .git_repos
        .list()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(repos.iter().map(GitRepo::from).collect()))
}

/// The configured git hosts (from settings), used to populate the repo form dropdown.
pub async fn list_git_hosts(
    State(state): State<AppState>,
    jar: CookieJar,
) -> Result<Json<Vec<GitHost>>, (StatusCode, String)> {
    require_admin(&state, &jar).await?;
    let hosts = state
        .settings
        .get(super::GIT_HOSTS_SETTING)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(default_git_hosts);
    Ok(Json(hosts))
}

pub async fn upsert_repo(
    State(state): State<AppState>,
    jar: CookieJar,
    Json(write): Json<GitRepoWrite>,
) -> Result<Json<GitRepo>, (StatusCode, String)> {
    let actor = require_admin(&state, &jar).await?;

    let identity = repo_identity(&write.host, &write.repo).ok_or((
        StatusCode::BAD_REQUEST,
        "invalid repository; expected a git host + repo path (e.g. github.com + org/repo)"
            .to_string(),
    ))?;
    let id = identity.id();
    let name = identity.path.clone();
    let url = identity.url();

    // Fetch the existing repo once (for "keep existing" semantics on omitted fields).
    let existing = state
        .git_repos
        .get(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // A provided read token is encrypted; otherwise the existing ciphertext is kept.
    let token_ciphertext = match write.token.as_deref() {
        Some(t) if !t.is_empty() => {
            let encrypted = {
                let cipher = state.cipher.read().await;
                cipher.encrypt(t)
            };
            Some(encrypted.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?)
        }
        _ => existing.as_ref().and_then(|r| r.token_ciphertext.clone()),
    };

    // The write token is encrypted the same way; an omitted value keeps the existing.
    let write_token_ciphertext = match write.write_token.as_deref() {
        Some(t) if !t.is_empty() => {
            let encrypted = {
                let cipher = state.cipher.read().await;
                cipher.encrypt(t)
            };
            Some(encrypted.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?)
        }
        _ => existing.as_ref().and_then(|r| r.write_token_ciphertext.clone()),
    };

    let entity = GitRepoEntity {
        id,
        name,
        url,
        token_ciphertext,
        write_token_ciphertext,
        enabled: write.enabled,
    };
    let saved = state
        .git_repos
        .save(&entity)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    audit(&state, &actor, "repo.upsert", "repo", &saved.id).await;
    Ok(Json(GitRepo::from(&saved)))
}

pub async fn delete_repo(
    State(state): State<AppState>,
    jar: CookieJar,
    Query(query): Query<DeleteRepoQuery>,
) -> Result<StatusCode, (StatusCode, String)> {
    let actor = require_admin(&state, &jar).await?;
    let id = query.id;
    state
        .git_repos
        .delete(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    audit(&state, &actor, "repo.delete", "repo", &id).await;
    Ok(StatusCode::NO_CONTENT)
}

// --- encryption key rotation -----------------------------------------------

/// Rotate the encryption key: generate a fresh key, re-encrypt every stored git
/// token under it, write the new key to its Secret, and swap the in-memory cipher.
pub async fn rotate_encryption_key(
    State(state): State<AppState>,
    jar: CookieJar,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let actor = require_admin(&state, &jar).await?;

    let store = state.key_store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "encryption key store not configured".to_string(),
    ))?;

    let old_key = store.load().await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let old_cipher = old_key
        .as_deref()
        .map(super::crypto::AesGcmCipher::from_base64_key)
        .transpose()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let new_key = super::crypto::AesGcmCipher::generate_key_base64()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let new_cipher = super::crypto::AesGcmCipher::from_base64_key(&new_key)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    // Phase 1: decrypt + re-encrypt every stored token IN MEMORY (no persistence yet).
    // If any step fails here, nothing has been written and the old key is untouched.
    let mut reencrypted: Vec<GitRepoEntity> = Vec::new();
    if let Some(old) = &old_cipher {
        let repos = state
            .git_repos
            .list()
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        for repo in repos {
            if let Some(ct) = &repo.token_ciphertext {
                let plaintext =
                    old.decrypt(ct).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
                let new_ct =
                    new_cipher.encrypt(&plaintext).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
                let mut updated = repo.clone();
                updated.token_ciphertext = Some(new_ct);
                reencrypted.push(updated);
            }
        }
    }

    // Phase 2: persist the new key FIRST — a failure here (e.g. RBAC) leaves both
    // the Secret and the stored tokens unchanged.
    store.store(&new_key).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Phase 3: save the re-encrypted tokens under the now-persisted new key.
    for updated in reencrypted {
        state
            .git_repos
            .save(&updated)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    // Phase 4: make the new key the active in-memory cipher.
    *state.cipher.write().await = new_cipher;

    audit(&state, &actor, "encryption-key.rotate", "settings", "encryption_key").await;
    Ok(Json(serde_json::json!({ "ok": true })))
}

// --- webhook secret --------------------------------------------------------

pub async fn get_webhook_secret(
    State(state): State<AppState>,
    jar: CookieJar,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    require_admin(&state, &jar).await?;
    let secret = state
        .settings
        .get(super::WEBHOOK_SECRET_KEY)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(serde_json::json!({ "secret": secret })))
}

pub async fn regenerate_webhook_secret(
    State(state): State<AppState>,
    jar: CookieJar,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let actor = require_admin(&state, &jar).await?;
    let secret = new_webhook_secret();
    state
        .settings
        .set(super::WEBHOOK_SECRET_KEY, &secret)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    audit(&state, &actor, "webhook-secret.regenerate", "settings", "webhook_secret").await;
    Ok(Json(serde_json::json!({ "secret": secret })))
}
