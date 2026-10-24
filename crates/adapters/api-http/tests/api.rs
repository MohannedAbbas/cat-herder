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

#[tokio::test]
async fn webhook_without_runtime_returns_503() {
    let (status, _) = post_json(
        app(),
        "/v1/webhooks/generic",
        r#"{"task_spec":"from webhook","dedup_key":"delivery-1"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

// --- mcp ------------------------------------------------------------------

#[tokio::test]
async fn mcp_initialize_returns_server_info() {
    let (status, body) = post_json(
        app(),
        "/mcp",
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_slice(&body).expect("valid JSON");
    assert_eq!(v["result"]["serverInfo"]["name"], "cat-herder");
}

#[tokio::test]
async fn mcp_tools_list_returns_tools() {
    let (status, body) = post_json(
        app(),
        "/mcp",
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_slice(&body).expect("valid JSON");
    let tools = v["result"]["tools"].as_array().expect("tools array");
    assert!(tools.iter().any(|t| t["name"] == "run_agent"));
    assert!(tools.iter().any(|t| t["name"] == "list_runs"));
}

#[tokio::test]
async fn mcp_list_agents_tool_works() {
    let (status, body) = post_json(
        app(),
        "/mcp",
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list_agents","arguments":{}}}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_slice(&body).expect("valid JSON");
    assert_eq!(v["result"]["isError"], false);
    let text = v["result"]["content"][0]["text"].as_str().unwrap();
    let agents: serde_json::Value = serde_json::from_str(text).unwrap();
    assert!(agents.as_array().unwrap().len() >= 3);
}

// --- run events ------------------------------------------------------------

#[tokio::test]
async fn post_finished_event_marks_run_succeeded() {
    use cat_herder_domain::entities::{Run, RunId, RunKind, RunState, TenantId};
    use cat_herder_domain::ports::RunRepository;
    use std::time::SystemTime;

    let state = AppState::default();
    let seeded = state
        .repo
        .save(&Run {
            id: RunId(0),
            tenant: TenantId(1),
            kind: RunKind::Agent,
            state: RunState::Running,
            task_spec: "event test".into(),
            session_id: None,
            max_concurrency: 1,
            created_at: SystemTime::now(),
            trigger: "api".into(),
            actor: None,
            model: None,
            repo: None,
            language: None,
            started_at: None,
            finished_at: None,
        })
        .await
        .unwrap();
    let app = router(state);

    let (status, body) = post_json(
        app,
        &format!("/v1/runs/{}/events", seeded.id.0),
        r#"{"type":"finished","status":"succeeded"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_slice(&body).expect("valid JSON");
    assert_eq!(v["state"], "succeeded");
    assert!(v["finishedAt"].is_string());
}

#[tokio::test]
async fn post_finished_event_failed_marks_run_failed() {
    use cat_herder_domain::entities::{Run, RunId, RunKind, RunState, TenantId};
    use cat_herder_domain::ports::RunRepository;
    use std::time::SystemTime;

    let state = AppState::default();
    let seeded = state
        .repo
        .save(&Run {
            id: RunId(0),
            tenant: TenantId(1),
            kind: RunKind::Agent,
            state: RunState::Running,
            task_spec: "event test".into(),
            session_id: None,
            max_concurrency: 1,
            created_at: SystemTime::now(),
            trigger: "api".into(),
            actor: None,
            model: None,
            repo: None,
            language: None,
            started_at: None,
            finished_at: None,
        })
        .await
        .unwrap();
    let app = router(state);

    let (status, body) = post_json(
        app,
        &format!("/v1/runs/{}/events", seeded.id.0),
        r#"{"type":"finished","status":"failed"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_slice(&body).expect("valid JSON");
    assert_eq!(v["state"], "failed");
}

#[tokio::test]
async fn post_event_unknown_run_returns_404() {
    let (status, _) = post_json(
        app(),
        "/v1/runs/999/events",
        r#"{"type":"finished","status":"succeeded"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn post_nonterminal_event_is_acknowledged_without_state_change() {
    use cat_herder_domain::entities::{Run, RunId, RunKind, RunState, TenantId};
    use cat_herder_domain::ports::RunRepository;
    use std::time::SystemTime;

    let state = AppState::default();
    let seeded = state
        .repo
        .save(&Run {
            id: RunId(0),
            tenant: TenantId(1),
            kind: RunKind::Agent,
            state: RunState::Running,
            task_spec: "event test".into(),
            session_id: None,
            max_concurrency: 1,
            created_at: SystemTime::now(),
            trigger: "api".into(),
            actor: None,
            model: None,
            repo: None,
            language: None,
            started_at: None,
            finished_at: None,
        })
        .await
        .unwrap();
    let app = router(state);

    let (status, body) = post_json(
        app,
        &format!("/v1/runs/{}/events", seeded.id.0),
        r#"{"type":"started"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_slice(&body).expect("valid JSON");
    assert_eq!(v["state"], "running");
}

// --- auth / admin ----------------------------------------------------------

async fn login(app: Router) -> (StatusCode, String) {
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"username":"admin","password":"admin"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let cookie = res
        .headers()
        .get("set-cookie")
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default();
    (status, cookie)
}

#[tokio::test]
async fn login_returns_session_cookie() {
    let (status, cookie) = login(app()).await;
    assert_eq!(status, StatusCode::OK);
    assert!(cookie.contains("session="), "cookie was: {cookie}");
}

#[tokio::test]
async fn login_rejects_bad_credentials() {
    let res = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"username":"admin","password":"wrong"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_endpoint_requires_auth() {
    let res = app()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/admin/providers")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_upsert_provider_with_session() {
    let app = app();
    let (_, cookie) = login(app.clone()).await;

    let res = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/v1/admin/providers/deepseek")
                .header("content-type", "application/json")
                .header("cookie", cookie)
                .body(Body::from(
                    r#"{"id":"deepseek","name":"DeepSeek","baseUrl":"https://api.deepseek.com","apiKeySecret":"cat-herder-deepseek","apiKeySecretKey":"api-key","enabled":true}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.unwrap().to_bytes().to_vec();
    let v: serde_json::Value = serde_json::from_slice(&body).expect("valid JSON");
    assert_eq!(v["id"], "deepseek");
    assert_eq!(v["apiKeySecret"], "cat-herder-deepseek");
}
