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

use anyhow::{Context, Result};
use axum::extract::State;
use axum::routing::get;
use axum::Router;
use tokio::task::JoinHandle;

use crate::metrics::AppMetrics;
use crate::shutdown::shutdown_signal;

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

/// Bind the internal server on `bind_addr`, then spawn it. The bind happens *before* the spawn
/// and its failure is returned to the caller (which fails startup), rather than panicking inside
/// a detached task where the process would otherwise keep running without `/metrics`. The spawned
/// server drains on `SIGTERM`/Ctrl-C via `with_graceful_shutdown`.
pub async fn spawn_internal_server(
    bind_addr: String,
    app_metrics: AppMetrics,
) -> Result<JoinHandle<()>> {
    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .with_context(|| format!("bind internal metrics server to {bind_addr}"))?;
    tracing::info!(addr = %bind_addr, "internal metrics server listening");
    Ok(tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, internal_router(app_metrics))
            .with_graceful_shutdown(shutdown_signal())
            .await
        {
            tracing::error!(error = %e, "internal metrics server error");
        }
    }))
}
