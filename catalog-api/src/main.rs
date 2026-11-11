//! `catalog-api` binary for Apps Platform (Cloud Run).
//!
//! One HTTP service on a single port: the Lance REST Catalog routes backed by Postgres, health
//! probes, and the operator UI.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;

use catalog_api_lib::config::AppConfig;
use catalog_api_lib::secrets::fetch_aws_secret_opts;
use catalog_api_lib::shutdown::shutdown_signal;
use catalog_store::PostgresNamespace;

/// How long boot keeps retrying the database; a Cloud SQL Auth Proxy sidecar may still be
/// starting when the app does.
const DB_CONNECT_DEADLINE: Duration = Duration::from_secs(30);

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
    // this is a no-op and the AWS_* env is used. They only open datasets server-side.
    let storage_options = fetch_aws_secret_opts()
        .await?
        .into_iter()
        .map(|(k, v)| (k.to_ascii_lowercase(), v))
        .collect();

    let started = Instant::now();
    let pool = loop {
        match catalog_store::connect(&cfg.database_url, cfg.db_schema.as_deref()).await {
            Ok(pool) => break pool,
            Err(e) if started.elapsed() < DB_CONNECT_DEADLINE => {
                tracing::warn!(error = %e, "database not ready; retrying");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Err(e) => return Err(e).context("connect to database and run migrations"),
        }
    };
    let namespace = Arc::new(
        PostgresNamespace::new(pool, storage_options)
            .with_top_level_allowlist(cfg.verticals)
            .context("CATALOG_VERTICALS")?,
    );

    let listener = tokio::net::TcpListener::bind(&cfg.bind_addr)
        .await
        .with_context(|| format!("bind api server to {}", cfg.bind_addr))?;
    tracing::info!(addr = %cfg.bind_addr, "catalog-api listening");

    axum::serve(
        listener,
        catalog_api_lib::app(namespace, cfg.catalog, Some(&cfg.webui_dir)),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .context("api server error")?;

    tracing::info!("shutdown complete");
    Ok(())
}
