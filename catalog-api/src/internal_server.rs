//! Internal HTTP server for operator/monitoring endpoints.
//!
//! Runs on a separate port from the main API server (default `0.0.0.0:9090`, configurable
//! via `CATALOG_METRICS_BIND_ADDR`). Exposes:
//!
//! - `GET /metrics` — Prometheus text-format exposition of all registered metrics.
//! - `GET /healthz` — liveness probe (same plain-`ok` as the main server's `/healthz`).
//!
//! ## Why a separate port?
//!
//! Design §12.2 specifies that `/metrics` should be on an internal-only port so that
//! network policy can allow Prometheus to scrape it without opening the endpoint to general
//! client traffic. The ServiceMonitor in `deploy/base/monitoring/` targets this `metrics`
//! port (`9090`), not the main API port (`8080`).
//!
//! In a real deployment, a `NetworkPolicy` would allow ingress on port 9090 only from the
//! Prometheus namespace, while the main API port is accessible to regular callers.

use axum::extract::State;
use axum::routing::get;
use axum::Router;
use tokio::task::JoinHandle;

use crate::metrics::AppMetrics;

async fn healthz() -> &'static str {
    "ok"
}

async fn metrics_handler(
    State(m): State<AppMetrics>,
) -> (
    axum::http::StatusCode,
    [(&'static str, &'static str); 1],
    String,
) {
    (
        axum::http::StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
        m.handle.render(),
    )
}

pub fn internal_router(app_metrics: AppMetrics) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/metrics", get(metrics_handler))
        .with_state(app_metrics)
}

/// Spawn the internal server on `bind_addr`. Returns the `JoinHandle`; drop it to cancel.
pub fn spawn_internal_server(bind_addr: String, app_metrics: AppMetrics) -> JoinHandle<()> {
    tokio::spawn(async move {
        let listener = tokio::net::TcpListener::bind(&bind_addr)
            .await
            .unwrap_or_else(|e| panic!("failed to bind internal server to {bind_addr}: {e}"));
        tracing::info!(addr = %bind_addr, "internal metrics server listening");
        axum::serve(listener, internal_router(app_metrics))
            .await
            .expect("internal metrics server error");
    })
}
