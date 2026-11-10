//! `catalog-api` library: the Lance REST Catalog HTTP surface, config, and bootstrap helpers,
//! shared between the `catalog-api` binary and its integration tests.

pub mod config;
pub mod rest;
pub mod secrets;
pub mod shutdown;
pub mod webui;

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use axum::Router;
use catalog_store::PostgresNamespace;

/// The full service: health probes, the `/v1` spec routes, and the SPA when `webui_dir` holds a
/// build.
pub fn app(namespace: Arc<PostgresNamespace>, webui_dir: Option<&str>) -> Router {
    let mut router = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(readyz).with_state(namespace.clone()))
        .nest("/v1", rest::router(namespace));
    if let Some(ui) = webui_dir.and_then(webui::router) {
        router = router.merge(ui);
    }
    router
}

/// Readiness tracks database reachability; liveness (`/healthz`) is `ok` from boot.
async fn readyz(State(namespace): State<Arc<PostgresNamespace>>) -> StatusCode {
    if namespace.ping().await {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}
