//! `catalog-api` binary for Apps Platform (Cloud Run).
//!
//! One stateless HTTP service on a single port: serves the REST API from the merged read model
//! (derived snapshot + authored overlay), and runs the sweep on demand via
//! `POST /internal/jobs/sweep` (triggered by Cloud Scheduler — there is no background loop
//! because Cloud Run throttles CPU between requests). All catalog state lives on S3.

use std::sync::Arc;

use anyhow::Context;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use tokio::sync::Mutex;

use catalog_api_lib::api::{api_router, ApiState};
use catalog_api_lib::catalog::Catalog;
use catalog_api_lib::config::AppConfig;
use catalog_api_lib::secrets::fetch_aws_secret_opts;
use catalog_api_lib::shutdown::shutdown_signal;
use catalog_api_lib::sweep::run_sweep;
use catalog_api_lib::sweep_config::{build_meta_store, build_sweep_config};
use catalog_api_lib::webui;
use catalog_store::SweepConfig;

async fn healthz() -> &'static str {
    "ok"
}

/// Readiness: 200 once the catalog view has loaded at least once, 503 before. Liveness
/// (`/healthz`) is `ok` from boot.
async fn readyz(State(catalog): State<Arc<Catalog>>) -> StatusCode {
    if catalog.is_ready().await {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}

#[derive(Clone)]
struct SweepState {
    sweep_cfg: SweepConfig,
    registry_path: String,
    meta: catalog_store::MetaStore,
    catalog: Arc<Catalog>,
    /// Serializes sweeps on this instance so a Cloud Scheduler double-fire doesn't run two at
    /// once (cross-instance concurrency is safe anyway: the snapshot is last-wins).
    lock: Arc<Mutex<()>>,
}

/// `POST /internal/jobs/sweep` — run one sweep. Idempotent; returns 2xx on success (Cloud
/// Scheduler retries non-2xx). Auth is enforced by the platform (Scheduler uses the app's OIDC
/// identity); this endpoint does no app-level auth.
async fn run_sweep_endpoint(State(s): State<SweepState>) -> axum::response::Response {
    let _guard = s.lock.lock().await;
    match run_sweep(&s.sweep_cfg, &s.registry_path, &s.meta).await {
        Ok(report) => {
            s.catalog.invalidate().await;
            (StatusCode::OK, Json(report)).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "sweep failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "sweep failed").into_response()
        }
    }
}

fn app(
    api_state: ApiState,
    catalog: Arc<Catalog>,
    sweep_state: SweepState,
    webui_dir: &str,
) -> Router {
    let mut router = Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz).with_state(catalog))
        .route(
            "/internal/jobs/sweep",
            post(run_sweep_endpoint).with_state(sweep_state),
        )
        .merge(api_router(api_state));
    // Serve the SPA from the same origin (no CORS) when its build output is present.
    if let Some(ui) = webui::router(webui_dir) {
        router = router.merge(ui);
    }
    router
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // JSON logs for Cloud Logging. Honor RUST_LOG when set; otherwise default to `info` but quiet
    // Lance's very chatty per-operation INFO events so they don't flood the log.
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,lance=warn"));
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(filter)
        .init();

    let cfg = AppConfig::from_env()?;

    // On Cloud Run the AWS keys come from Secret Manager (no ambient AWS credential); locally
    // this is a no-op and env/MinIO credentials are used. Fetched once, shared by both stores.
    let aws_opts = fetch_aws_secret_opts().await?;
    let sweep_cfg = build_sweep_config(&cfg.sweep_root_uri, &aws_opts)?;
    let meta = build_meta_store(&cfg.meta_base_uri, &aws_opts)?;
    let catalog = Catalog::new(cfg.registry_path.clone(), meta.clone(), cfg.cache_ttl);

    let api_state = ApiState::new(
        catalog.clone(),
        meta.clone(),
        sweep_cfg.clone(),
        cfg.ttl_audit_path.clone(),
    );
    let sweep_state = SweepState {
        sweep_cfg,
        registry_path: cfg.registry_path.clone(),
        meta,
        catalog: catalog.clone(),
        lock: Arc::new(Mutex::new(())),
    };

    let listener = tokio::net::TcpListener::bind(&cfg.bind_addr)
        .await
        .with_context(|| format!("bind api server to {}", cfg.bind_addr))?;
    tracing::info!(addr = %cfg.bind_addr, "catalog-api listening");

    axum::serve(
        listener,
        app(api_state, catalog, sweep_state, &cfg.webui_dir),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .context("api server error")?;

    tracing::info!("shutdown complete");
    Ok(())
}
