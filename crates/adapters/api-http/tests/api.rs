//! Integration tests for the HTTP API (`PLAN.md` §14).
//!
//! The `/v1` endpoint tests encode the expected contract and are currently **RED**
//! (stub handlers return 501); they go green once the handlers are implemented.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use cat_herder_api_http::{router, AppState};
use cat_herder_domain::api::{AgentTemplate, Session, SessionSummary};
use http_body_util::BodyExt;
use tower::ServiceExt;

fn app() -> Router {
    router(AppState::default())
}

async fn send(app: Router, uri: &str) -> (StatusCode, Vec<u8>) {
    let res = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let body = res.into_body().collect().await.unwrap().to_bytes().to_vec();
    (status, body)
}

#[tokio::test]
async fn healthz_returns_ok() {
    let (status, body) = send(app(), "/healthz").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(&body[..], b"ok");
}

#[tokio::test]
async fn list_sessions_returns_json_array() {
    let (status, body) = send(app(), "/v1/sessions").await;
    assert_eq!(status, StatusCode::OK);
    let sessions: Vec<SessionSummary> = serde_json::from_slice(&body).expect("valid JSON");
    assert!(!sessions.is_empty(), "expected at least one session");
    assert!(sessions.iter().all(|s| !s.id.is_empty()));
}

#[tokio::test]
async fn get_session_returns_json() {
    let (status, body) = send(app(), "/v1/sessions/sess-01").await;
    assert_eq!(status, StatusCode::OK);
    let session: Session = serde_json::from_slice(&body).expect("valid JSON");
    assert_eq!(session.id, "sess-01");
    assert!(!session.runs.is_empty());
}

#[tokio::test]
async fn get_unknown_session_returns_404() {
    let (status, _) = send(app(), "/v1/sessions/does-not-exist").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn list_agents_returns_json_array() {
    let (status, body) = send(app(), "/v1/agents").await;
    assert_eq!(status, StatusCode::OK);
    let agents: Vec<AgentTemplate> = serde_json::from_slice(&body).expect("valid JSON");
    assert!(!agents.is_empty(), "expected at least one agent");
    assert!(agents.iter().all(|a| !a.id.is_empty()));
}

#[tokio::test]
async fn get_agent_returns_json() {
    let (status, body) = send(app(), "/v1/agents/agent-codeact").await;
    assert_eq!(status, StatusCode::OK);
    let agent: AgentTemplate = serde_json::from_slice(&body).expect("valid JSON");
    assert_eq!(agent.id, "agent-codeact");
}

#[tokio::test]
async fn get_unknown_agent_returns_404() {
    let (status, _) = send(app(), "/v1/agents/does-not-exist").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// --- runs -----------------------------------------------------------------

async fn post_json(app: Router, uri: &str, body: &str) -> (StatusCode, Vec<u8>) {
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let body = res.into_body().collect().await.unwrap().to_bytes().to_vec();
    (status, body)
}

#[tokio::test]
async fn list_runs_starts_empty() {
    let (status, body) = send(app(), "/v1/runs").await;
    assert_eq!(status, StatusCode::OK);
    let runs: Vec<serde_json::Value> = serde_json::from_slice(&body).expect("valid JSON");
    assert!(runs.is_empty());
}

#[tokio::test]
async fn create_run_without_runtime_returns_503() {
    let (status, _) = post_json(
        app(),
        "/v1/runs",
        r#"{"task_spec":"hello","kind":"agent","max_concurrency":1}"#,
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn get_unknown_run_returns_404() {
    let (status, _) = send(app(), "/v1/runs/999").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
