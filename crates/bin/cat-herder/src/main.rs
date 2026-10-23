//! Cat Herder — control plane binary (composition root).
//!
//! Wires the runtime adapter into the API. Data is in-memory mock until Postgres
//! repositories are implemented.

use cat_herder_api_http::{router, AppState};
use cat_herder_runtime_agent_sandbox::SandboxRuntime;

#[tokio::main]
async fn main() {
    let namespace = std::env::var("CAT_HERDER_NAMESPACE").unwrap_or_else(|_| "cat-herder".into());
    let harness_image = std::env::var("CAT_HERDER_HARNESS_IMAGE")
        .unwrap_or_else(|_| "127.0.0.1:5000/cat-herder-agent-harness".into());

    // The runtime is optional at startup — if the cluster/client is unavailable we
    // still serve read endpoints (POST /v1/runs will return 503).
    let runtime = match SandboxRuntime::new(namespace, harness_image).await {
        Ok(rt) => Some(rt),
        Err(e) => {
            eprintln!("warning: execution runtime unavailable: {e}");
            None
        }
    };

    let state = AppState {
        runtime,
        ..AppState::default()
    };

    let app = router(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080")
        .await
        .expect("bind :8080");

    axum::serve(listener, app).await.expect("serve");
}
