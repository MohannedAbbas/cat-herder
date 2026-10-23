//! Cat Herder — control plane binary (composition root).
//!
//! Wires adapters to the domain ports. This skeleton serves the REST API from
//! in-memory mock data; real repositories land as adapters are built out.

use cat_herder_api_http::{router, AppState};

#[tokio::main]
async fn main() {
    let app = router(AppState::default());

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080")
        .await
        .expect("bind :8080");

    axum::serve(listener, app).await.expect("serve");
}
