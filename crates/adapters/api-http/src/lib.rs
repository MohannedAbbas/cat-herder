//! HTTP adapter (Axum): exposes the control-plane REST API (see `PLAN.md` §14).

use axum::{Router, routing::get};

/// Build the public router. Endpoints are added here as they are implemented.
pub fn router() -> Router {
    Router::new().route("/healthz", get(|| async { "ok" }))
}
