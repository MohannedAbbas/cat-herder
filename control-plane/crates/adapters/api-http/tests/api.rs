//! Integration tests for the HTTP API (`PLAN.md` §14).
//!
//! The `/v1` endpoint tests encode the expected contract and are currently **RED**
//! (stub handlers return 501); they go green once the handlers are implemented.

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use cat_herder_api_http::{
    AppState, GITHUB, GITLAB, GitHubOAuth, GitLabOAuth, OAuthConfig, router,
};
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
async fn get_run_events_unknown_run_returns_404() {
    let (status, _) = send(app(), "/v1/runs/999/events").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn get_run_events_replays_the_log() {
    use cat_herder_domain::entities::{Run, RunId, RunKind, RunState};
    use cat_herder_domain::events::Event;
    use cat_herder_domain::ports::{EventRepository, RunRepository};
    use std::time::SystemTime;

    let state = AppState::default();
    let run = Run {
        id: RunId(42),
        kind: RunKind::Agent,
        state: RunState::Queued,
        task_spec: "seed".into(),
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
        pr_url: None,
        report: None,
        input_tokens: 0,
        output_tokens: 0,
    };
    state.repo.save(&run).await.unwrap();
    state
        .events
        .append(run.id, &Event::run_queued(&run))
        .await
        .unwrap();
    state
        .events
        .append(run.id, &Event::run_scheduled(&run))
        .await
        .unwrap();

    let (status, body) = send(router(state), "/v1/runs/42/events").await;
    assert_eq!(status, StatusCode::OK);
    let list: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(list["events"].as_array().unwrap().len(), 2);
    assert_eq!(list["events"][0]["seq"], 1);
    assert_eq!(list["events"][0]["type"], "run.queued");
    assert_eq!(list["events"][1]["seq"], 2);
    assert_eq!(list["events"][1]["type"], "run.scheduled");
    assert_eq!(list["nextAfter"], 2);
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
    use cat_herder_domain::entities::{Run, RunId, RunKind, RunState};
    use cat_herder_domain::ports::RunRepository;
    use std::time::SystemTime;

    let state = AppState::default();
    let seeded = state
        .repo
        .save(&Run {
            id: RunId(0),
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
            pr_url: None,
            report: None,
            input_tokens: 0,
            output_tokens: 0,
        })
        .await
        .unwrap();
    let app = router(state);

    let (status, body) = post_json(
        app,
        &format!("/v1/runs/{}/events", seeded.id.0),
        r##"{"type":"finished","status":"succeeded","report":"# All done.\n"}"##,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_slice(&body).expect("valid JSON");
    assert_eq!(v["state"], "succeeded");
    assert!(v["finishedAt"].is_string());
    assert_eq!(v["report"], "# All done.\n");
}

#[tokio::test]
async fn post_finished_event_failed_marks_run_failed() {
    use cat_herder_domain::entities::{Run, RunId, RunKind, RunState};
    use cat_herder_domain::ports::RunRepository;
    use std::time::SystemTime;

    let state = AppState::default();
    let seeded = state
        .repo
        .save(&Run {
            id: RunId(0),
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
            pr_url: None,
            report: None,
            input_tokens: 0,
            output_tokens: 0,
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
async fn post_stuck_event_marks_run_stuck() {
    use cat_herder_domain::entities::{Run, RunId, RunKind, RunState};
    use cat_herder_domain::ports::RunRepository;
    use std::time::SystemTime;

    let state = AppState::default();
    let seeded = state
        .repo
        .save(&Run {
            id: RunId(0),
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
            pr_url: None,
            report: None,
            input_tokens: 0,
            output_tokens: 0,
        })
        .await
        .unwrap();
    let app = router(state);

    let (status, body) = post_json(
        app,
        &format!("/v1/runs/{}/events", seeded.id.0),
        r#"{"type":"stuck"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_slice(&body).expect("valid JSON");
    assert_eq!(v["state"], "stuck");
    assert!(v["finishedAt"].is_string());
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
    use cat_herder_domain::entities::{Run, RunId, RunKind, RunState};
    use cat_herder_domain::ports::RunRepository;
    use std::time::SystemTime;

    let state = AppState::default();
    let seeded = state
        .repo
        .save(&Run {
            id: RunId(0),
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
            pr_url: None,
            report: None,
            input_tokens: 0,
            output_tokens: 0,
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

#[tokio::test]
async fn admin_action_is_audited() {
    let app = app();
    let (_, cookie) = login(app.clone()).await;

    // mutate an agent (audited)
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/v1/admin/agents/agent-codeact")
                .header("content-type", "application/json")
                .header("cookie", cookie.clone())
                .body(Body::from(
                    r#"{"id":"agent-codeact","name":"CodeAct coding agent","description":"d","image":"i","runtimeTier":"default","model":"deepseek:deepseek-chat","tools":[],"maxConcurrency":8}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // list audit (audited + authenticated)
    let res = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/admin/audit")
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.unwrap().to_bytes().to_vec();
    let entries: serde_json::Value = serde_json::from_slice(&body).expect("valid JSON");
    assert!(
        entries
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["action"] == "agent.upsert"),
        "audit should contain the agent.upsert action"
    );
}

// --- webhook secret --------------------------------------------------------

#[tokio::test]
async fn webhook_requires_secret_header() {
    use cat_herder_domain::ports::SettingRepository;

    let state = AppState::default();
    state
        .settings
        .set("webhook_secret", "test-secret")
        .await
        .unwrap();
    let app = router(state);

    // missing header -> 401
    let (status, _) = post_json(app.clone(), "/v1/webhooks/generic", r#"{"task_spec":"x"}"#).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // wrong header -> 401
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/webhooks/generic")
                .header("content-type", "application/json")
                .header("x-webhook-secret", "wrong")
                .body(Body::from(r#"{"task_spec":"x"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // correct header but no runtime -> 503 (past the auth check)
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/webhooks/generic")
                .header("content-type", "application/json")
                .header("x-webhook-secret", "test-secret")
                .body(Body::from(r#"{"task_spec":"x"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn admin_can_generate_and_view_webhook_secret() {
    let app = app();
    let (_, cookie) = login(app.clone()).await;

    // initially null
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/admin/settings/webhook-secret")
                .header("cookie", cookie.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes().to_vec();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(v["secret"].is_null());

    // regenerate
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/admin/settings/webhook-secret")
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes().to_vec();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(v["secret"].as_str().is_some_and(|s| !s.is_empty()));
}

// --- admin CRUD (agents/models/tools/providers) ----------------------------

async fn authed(
    app: Router,
    method: &str,
    uri: &str,
    cookie: &str,
    body: &str,
) -> (StatusCode, Vec<u8>) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("cookie", cookie);
    if !body.is_empty() {
        builder = builder.header("content-type", "application/json");
    }
    let res = app
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let body = res.into_body().collect().await.unwrap().to_bytes().to_vec();
    (status, body)
}

#[tokio::test]
async fn admin_delete_agent() {
    let app = app();
    let (_, cookie) = login(app.clone()).await;
    let (status, _) = authed(app, "DELETE", "/v1/admin/agents/agent-codeact", &cookie, "").await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn admin_list_and_upsert_and_delete_model() {
    let app = app();
    let (_, cookie) = login(app.clone()).await;

    let (status, body) = authed(app.clone(), "GET", "/v1/admin/models", &cookie, "").await;
    assert_eq!(status, StatusCode::OK);
    let models: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(models.as_array().unwrap().len() >= 5);

    let (status, body) = authed(
        app.clone(),
        "PUT",
        "/v1/admin/models/gpt-4o",
        &cookie,
        r#"{"id":"gpt-4o","providerId":"openai","displayName":"GPT-4o"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["id"], "gpt-4o");

    let (status, _) = authed(app, "DELETE", "/v1/admin/models/gpt-4o", &cookie, "").await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn admin_list_and_upsert_and_delete_tool() {
    let app = app();
    let (_, cookie) = login(app.clone()).await;

    let (status, body) = authed(app.clone(), "GET", "/v1/admin/tools", &cookie, "").await;
    assert_eq!(status, StatusCode::OK);
    let tools: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(tools.as_array().unwrap().len() >= 6);

    let (status, body) = authed(
        app.clone(),
        "PUT",
        "/v1/admin/tools/browser",
        &cookie,
        r#"{"id":"browser","description":"Browser automation"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["id"], "browser");

    let (status, _) = authed(app, "DELETE", "/v1/admin/tools/browser", &cookie, "").await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn admin_delete_provider() {
    let app = app();
    let (_, cookie) = login(app.clone()).await;
    let (status, _) = authed(app, "DELETE", "/v1/admin/providers/openai", &cookie, "").await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

// --- mcp tools -------------------------------------------------------------

#[tokio::test]
async fn mcp_list_runs_and_sessions_tools() {
    let (status, body) = post_json(
        app(),
        "/mcp",
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"list_runs","arguments":{}}}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["result"]["isError"], false);

    let (status, body) = post_json(
        app(),
        "/mcp",
        r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"list_sessions","arguments":{}}}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["result"]["isError"], false);
}

#[tokio::test]
async fn mcp_run_agent_missing_task_spec() {
    let (status, body) = post_json(
        app(),
        "/mcp",
        r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"run_agent","arguments":{"kind":"agent"}}}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["result"]["isError"], true);
    let text = v["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("task_spec"));
}

#[tokio::test]
async fn mcp_run_agent_no_runtime() {
    let (status, body) = post_json(
        app(),
        "/mcp",
        r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"run_agent","arguments":{"task_spec":"hi","agent_id":"agent-codeact"}}}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["result"]["isError"], true);
    let text = v["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("runtime"));
}

// --- GitHub OAuth SSO ------------------------------------------------------

fn oauth_app() -> Router {
    let cfg = OAuthConfig {
        client_id: "test-client".into(),
        client_secret: "test-secret".into(),
        redirect_url: "https://cat.example.com".into(),
        allowed_users: vec!["octocat".into()],
        provider: &GITHUB,
        base_url: None,
    };
    let mut state = AppState::default();
    state.github_oauth = Some(GitHubOAuth::new(cfg));
    router(state)
}

#[tokio::test]
async fn github_login_redirects_to_github_and_sets_state_cookie() {
    let res = oauth_app()
        .oneshot(
            Request::builder()
                .uri("/v1/auth/github/login")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        res.status().is_redirection(),
        "expected 3xx, got {}",
        res.status()
    );
    let location = res
        .headers()
        .get(axum::http::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .expect("Location header");
    assert!(location.starts_with("https://github.com/login/oauth/authorize"));
    assert!(location.contains("client_id=test-client"));
    assert!(location.contains("state="));

    let set_cookie = res
        .headers()
        .get(axum::http::header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        set_cookie.contains("gh_oauth_state="),
        "missing CSRF cookie"
    );
    assert!(set_cookie.to_lowercase().contains("httponly"));
}

#[tokio::test]
async fn github_login_unconfigured_returns_404() {
    // Default AppState has github_oauth = None.
    let (status, _body) = send(app(), "/v1/auth/github/login").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// --- SSO provider discovery ------------------------------------------------

fn oauth_cfg(provider_kind: &str) -> OAuthConfig {
    OAuthConfig {
        client_id: "test-client".into(),
        client_secret: "test-secret".into(),
        redirect_url: "https://cat.example.com".into(),
        allowed_users: vec!["octocat".into()],
        provider: if provider_kind == "gitlab" {
            &GITLAB
        } else {
            &GITHUB
        },
        base_url: None,
    }
}

async fn sso_providers(app: Router) -> Vec<String> {
    let (status, body) = send(app, "/v1/auth/sso-providers").await;
    assert_eq!(status, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    v["providers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn sso_providers_empty_when_none_configured() {
    assert!(sso_providers(app()).await.is_empty());
}

#[tokio::test]
async fn sso_providers_lists_github_only() {
    let mut state = AppState::default();
    state.github_oauth = Some(GitHubOAuth::new(oauth_cfg("github")));
    assert_eq!(sso_providers(router(state)).await, vec!["github"]);
}

#[tokio::test]
async fn sso_providers_lists_both_when_configured() {
    let mut state = AppState::default();
    state.github_oauth = Some(GitHubOAuth::new(oauth_cfg("github")));
    state.gitlab_oauth = Some(GitLabOAuth::new(oauth_cfg("gitlab")));
    assert_eq!(sso_providers(router(state)).await, vec!["github", "gitlab"]);
}

#[tokio::test]
async fn sso_providers_reports_both_when_mock_enabled() {
    // Mock mode pretends both providers are configured without any OAuth config.
    let mut state = AppState::default();
    state.mock_oauth = Some("dev-user".into());
    assert_eq!(sso_providers(router(state)).await, vec!["github", "gitlab"]);
}

#[tokio::test]
async fn github_login_mock_issues_session_and_redirects_home() {
    let mut state = AppState::default();
    state.mock_oauth = Some("dev-user".into());
    // Share the in-memory session store between the login and the follow-up /me call.
    let shared = state.clone();
    let res = router(state)
        .oneshot(
            Request::builder()
                .uri("/v1/auth/github/login")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Redirects to `/` (same as a successful callback), not to the provider.
    assert!(
        res.status().is_redirection(),
        "expected 3xx, got {}",
        res.status()
    );
    assert_eq!(
        res.headers()
            .get(axum::http::header::LOCATION)
            .and_then(|v| v.to_str().ok()),
        Some("/"),
    );

    // A session cookie was issued.
    let set_cookie = res
        .headers()
        .get(axum::http::header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(set_cookie.contains("session="), "missing session cookie");
    let token = set_cookie
        .split(';')
        .next()
        .and_then(|kv| kv.strip_prefix("session="))
        .expect("session token")
        .to_string();

    // The session is valid and reports the mock username via /v1/auth/me.
    let res = router(shared)
        .oneshot(
            Request::builder()
                .uri("/v1/auth/me")
                .header(axum::http::header::COOKIE, format!("session={token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["authenticated"], serde_json::Value::Bool(true));
    assert_eq!(v["username"], serde_json::Value::String("dev-user".into()));
}

#[tokio::test]
async fn github_callback_rejects_missing_state_cookie() {
    // No gh_oauth_state cookie => CSRF check fails before any GitHub call.
    let (status, _body) = send(oauth_app(), "/v1/auth/github/callback?code=abc&state=xyz").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn github_callback_unconfigured_returns_404() {
    let (status, _body) = send(app(), "/v1/auth/github/callback?code=abc&state=xyz").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// --- GitLab OAuth SSO ------------------------------------------------------

fn gitlab_oauth_app() -> Router {
    let cfg = OAuthConfig {
        client_id: "test-client".into(),
        client_secret: "test-secret".into(),
        redirect_url: "https://cat.example.com".into(),
        allowed_users: vec!["octocat".into()],
        provider: &GITLAB,
        base_url: None,
    };
    let mut state = AppState::default();
    state.gitlab_oauth = Some(GitLabOAuth::new(cfg));
    router(state)
}

#[tokio::test]
async fn gitlab_login_redirects_to_gitlab_and_sets_state_cookie() {
    let res = gitlab_oauth_app()
        .oneshot(
            Request::builder()
                .uri("/v1/auth/gitlab/login")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        res.status().is_redirection(),
        "expected 3xx, got {}",
        res.status()
    );
    let location = res
        .headers()
        .get(axum::http::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .expect("Location header");
    assert!(location.starts_with("https://gitlab.com/oauth/authorize"));
    assert!(location.contains("client_id=test-client"));
    assert!(location.contains("scope=read_user"));
    assert!(location.contains("state="));

    let set_cookie = res
        .headers()
        .get(axum::http::header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        set_cookie.contains("gl_oauth_state="),
        "missing CSRF cookie"
    );
    assert!(set_cookie.to_lowercase().contains("httponly"));
}

#[tokio::test]
async fn gitlab_login_unconfigured_returns_404() {
    let (status, _body) = send(app(), "/v1/auth/gitlab/login").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn gitlab_callback_rejects_missing_state_cookie() {
    let (status, _body) = send(
        gitlab_oauth_app(),
        "/v1/auth/gitlab/callback?code=abc&state=xyz",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn gitlab_callback_unconfigured_returns_404() {
    let (status, _body) = send(app(), "/v1/auth/gitlab/callback?code=abc&state=xyz").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
