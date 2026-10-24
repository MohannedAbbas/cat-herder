//! Admin + auth HTTP handlers (see `PLAN.md` §3/§14). Everything under `/v1/admin`
//! requires an authenticated admin session (HttpOnly cookie).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use axum_extra::extract::cookie::{Cookie, CookieJar};
use serde::Deserialize;

use cat_herder_domain::api::{AgentTemplate, AuditEntry, Model, Provider, Tool};
use cat_herder_domain::ports::{
    AgentRepository, AuditRepository, ModelRepository, ProviderRepository, ToolRepository,
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
            let cookie = Cookie::build(("session", token))
                .http_only(true)
                .same_site(axum_extra::extract::cookie::SameSite::Lax)
                .path("/")
                .build();
            Ok((
                StatusCode::OK,
                jar.add(cookie),
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
    let jar = jar;
    if let Some(c) = jar.get("session") {
        state.auth.logout(c.value()).await;
    }
    let removal = Cookie::build(("session", "")).path("/").build();
    (StatusCode::OK, jar.remove(removal))
}

pub async fn me(State(state): State<AppState>, jar: CookieJar) -> Json<serde_json::Value> {
    let token = jar.get("session").map(|c| c.value().to_string());
    let authenticated = match token {
        Some(t) => state.auth.valid_session(&t).await,
        None => false,
    };
    Json(serde_json::json!({
        "authenticated": authenticated,
        "username": if authenticated { serde_json::json!("admin") } else { serde_json::Value::Null },
    }))
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
