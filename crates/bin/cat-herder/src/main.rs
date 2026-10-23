//! Cat Herder — control plane binary (composition root).
//!
//! Wires adapters to the domain ports. This skeleton serves only `/healthz`;
//! real wiring lands as adapters are built out.

use axum::Router;

#[tokio::main]
async fn main() {
    let app = Router::new().merge(cat_herder_api_http::router());

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080")
        .await
        .expect("bind :8080");

    axum::serve(listener, app).await.expect("serve");
}
