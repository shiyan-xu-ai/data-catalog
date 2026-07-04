use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use axum::extract::State;
use axum::http::StatusCode;
use axum::{routing::get, Json, Router};
use tokio::sync::RwLock;
use tower_http::cors::CorsLayer;

use catalog_api_lib::api::{api_router, ApiState};
use catalog_api_lib::config::{AppConfig, LeaderMode};
use catalog_api_lib::internal_server;
use catalog_api_lib::leader::{
    self, ForcedLeaderElector, KubeLeaseElector, LeaderElector, LeaderState,
};
use catalog_api_lib::metrics::{self, AppMetrics};
use catalog_api_lib::registry_cache::{self, Hydrated, RegistryCache};
use catalog_api_lib::registry_lock::{self, RegistryWriteLock};
use catalog_api_lib::shutdown::shutdown_signal;
use catalog_api_lib::sweep_config;
use catalog_api_lib::sweep_loop;
use catalog_store::SweepConfig;

async fn healthz() -> &'static str {
    "ok"
}

/// Readiness probe: 200 once the pod has completed at least one successful registry read (so it
/// serves a cache that reflects storage), 503 before that. Distinct from `/healthz` (liveness),
/// which is `ok` from boot — a fresh pod that hasn't hydrated yet is alive but not ready.
async fn readyz(State(hydrated): State<Hydrated>) -> StatusCode {
    if hydrated.load(Ordering::SeqCst) {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}

/// Internal debug endpoint: inspect the currently cached registry state. Not part of the
/// public REST API — useful for local dev and integration tests.
async fn debug_registry(State(cache): State<RegistryCache>) -> Json<Vec<catalog_core::TableEntry>> {
    Json(cache.read().await.clone())
}

async fn debug_is_leader(State(state): State<LeaderState>) -> Json<bool> {
    Json(leader::is_leader(&state))
}

#[allow(clippy::too_many_arguments)]
fn app(
    registry_cache: RegistryCache,
    hydrated: Hydrated,
    leader_state: LeaderState,
    registry_path: String,
    write_lock: RegistryWriteLock,
    sweep_cfg: SweepConfig,
    ttl_audit_path: String,
) -> Router {
    let api_state = ApiState::new(
        registry_path,
        registry_cache.clone(),
        leader_state.clone(),
        write_lock,
        sweep_cfg,
        ttl_audit_path,
    );
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz).with_state(hydrated))
        .route(
            "/debug/registry",
            get(debug_registry).with_state(registry_cache),
        )
        .route(
            "/debug/is_leader",
            get(debug_is_leader).with_state(leader_state),
        )
        .merge(api_router(api_state))
        // Permissive CORS for the public /v1 + /ext surface.
        .layer(CorsLayer::permissive())
}

async fn build_leader_elector(cfg: &AppConfig) -> anyhow::Result<Arc<dyn LeaderElector>> {
    match &cfg.leader_mode {
        LeaderMode::Forced(leader) => Ok(Arc::new(ForcedLeaderElector::new(*leader))),
        LeaderMode::Kube {
            namespace,
            lease_name,
            holder_identity,
        } => {
            let elector = KubeLeaseElector::from_env(
                namespace,
                lease_name.clone(),
                holder_identity.clone(),
                cfg.lease_duration,
            )
            .await?;
            Ok(Arc::new(elector))
        }
    }
}

/// Polls leader state every 5 s and updates the `catalog_is_leader` gauge.
fn spawn_leader_gauge_updater(leader_state: LeaderState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            metrics::set_is_leader(leader::is_leader(&leader_state));
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    })
}

/// Polls the hydration flag every 5 s and sets the `catalog_hydration_ready` gauge. Uses the
/// same signal as `/readyz` (first successful registry read), so gauge and probe agree.
fn spawn_hydration_gauge_updater(hydrated: Hydrated) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            metrics::set_hydration_ready(hydrated.load(Ordering::SeqCst));
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    })
}

/// Polls `last_sweep_at` every 5 s and updates `catalog_snapshot_staleness_seconds`.
///
/// Only the leader sweeps, so freshness is a leader-only SLI: while leader, reports seconds
/// since the last successful write (or since process start if none yet, so the gauge climbs
/// from boot and `FreshnessBreach` fires if the first sweep never lands). While NOT leader it
/// reports 0 — otherwise a pod that stepped down would keep reporting its last (growing) value
/// and trip `FreshnessBreach` even though it is no longer responsible for sweeping.
fn spawn_staleness_gauge_updater(
    last_sweep_at: catalog_api_lib::sweep_loop::LastSweepAt,
    process_start: std::time::Instant,
    leader_state: LeaderState,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let staleness = if leader::is_leader(&leader_state) {
                let guard = last_sweep_at.lock().await;
                match *guard {
                    Some(t) => t.elapsed().as_secs_f64(),
                    None => process_start.elapsed().as_secs_f64(),
                }
            } else {
                0.0
            };
            metrics::set_snapshot_staleness(staleness);
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    })
}

/// Periodically runs the Prometheus recorder's upkeep (idle-metric reclamation, histogram
/// rotation) so exposition memory does not depend solely on scrape cadence.
fn spawn_metrics_upkeep(app_metrics: AppMetrics) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            app_metrics.handle.run_upkeep();
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    })
}

/// Periodically collects process metrics (RSS, CPU, open fds) into the recorder. `describe()`
/// only registers metadata; `collect()` is what actually emits the `process_*` samples.
fn spawn_process_metrics_collector(
    collector: metrics_process::Collector,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            collector.collect();
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    })
}

/// Collects tokio runtime metrics at 15-second intervals and records them via the `metrics`
/// facade under `tokio_*` names.
fn spawn_tokio_metrics_collector(
    runtime_monitor: tokio_metrics::RuntimeMonitor,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut intervals = runtime_monitor.intervals();
        loop {
            if let Some(interval) = intervals.next() {
                ::metrics::gauge!("tokio_workers_count").set(interval.workers_count as f64);
                ::metrics::gauge!("tokio_live_tasks_count").set(interval.live_tasks_count as f64);
                ::metrics::gauge!("tokio_worker_total_busy_duration_seconds")
                    .set(interval.total_busy_duration.as_secs_f64());
                ::metrics::counter!("tokio_total_park_count_total")
                    .absolute(interval.total_park_count);
            }
            tokio::time::sleep(Duration::from_secs(15)).await;
        }
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let process_start = std::time::Instant::now();

    tracing_subscriber::fmt::init();

    // Install the global Prometheus recorder before any metrics::* calls.
    let app_metrics: AppMetrics = metrics::init();

    // Periodically drain the recorder's upkeep queue (histogram bucket rotation etc.), else
    // memory is bounded only by scrape frequency.
    let _metrics_upkeep = spawn_metrics_upkeep(app_metrics.clone());

    // Register AND periodically collect process metrics (RSS, CPU, open fds): describing alone
    // never emits any samples, so the `process_*` series must be collected on an interval.
    let process_collector = metrics_process::Collector::default();
    process_collector.describe();
    let _process_metrics = spawn_process_metrics_collector(process_collector);

    let cfg = AppConfig::from_env()?;

    let leader_state: LeaderState = Arc::new(AtomicBool::new(false));
    let elector = build_leader_elector(&cfg).await?;
    let leader_task = leader::run_leader_election(
        elector.clone(),
        leader_state.clone(),
        cfg.leader_tick_interval,
        cfg.lease_duration,
    );

    let registry_cache: RegistryCache = Arc::new(RwLock::new(Vec::new()));
    let hydrated: Hydrated = Arc::new(AtomicBool::new(false));
    let _refresh_task = registry_cache::spawn_refresh(
        cfg.registry_path.clone(),
        registry_cache.clone(),
        hydrated.clone(),
        cfg.registry_refresh_interval,
    );

    // Shared with the sweep loop below: the SAME lock guards every writer's
    // read-registry -> mutate -> write-registry critical section.
    let write_lock: RegistryWriteLock = registry_lock::new_registry_write_lock();

    let last_sweep_at = sweep_loop::new_last_sweep_at();

    let sweep_cfg = sweep_config::build_sweep_config(&cfg.sweep_root_uri)?;
    let _sweep_task = sweep_loop::spawn_sweep_loop(
        sweep_cfg.clone(),
        cfg.registry_path.clone(),
        leader_state.clone(),
        write_lock.clone(),
        last_sweep_at.clone(),
        cfg.sweep_interval,
    );

    // Background gauge updaters.
    let _leader_gauge = spawn_leader_gauge_updater(leader_state.clone());
    let _hydration_gauge = spawn_hydration_gauge_updater(hydrated.clone());
    let _staleness_gauge =
        spawn_staleness_gauge_updater(last_sweep_at, process_start, leader_state.clone());

    // Tokio runtime metrics collector.
    let runtime_monitor = tokio_metrics::RuntimeMonitor::new(&tokio::runtime::Handle::current());
    let _tokio_metrics = spawn_tokio_metrics_collector(runtime_monitor);

    // Internal server: /metrics + /healthz on a separate network-policy-restricted port. Bound
    // synchronously here so a bind failure fails startup instead of being swallowed in a task.
    let _internal_task =
        internal_server::spawn_internal_server(cfg.metrics_bind_addr.clone(), app_metrics).await?;

    let listener = tokio::net::TcpListener::bind(&cfg.bind_addr)
        .await
        .with_context(|| format!("bind api server to {}", cfg.bind_addr))?;
    axum::serve(
        listener,
        app(
            registry_cache,
            hydrated,
            leader_state.clone(),
            cfg.registry_path.clone(),
            write_lock,
            sweep_cfg,
            cfg.ttl_audit_path.clone(),
        ),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .context("api server error")?;

    // The server has drained on SIGTERM/Ctrl-C. Stop the election loop (so it won't renew) and,
    // if we were the leader, relinquish the lease so a successor takes over promptly instead of
    // waiting a full lease_duration for expiry.
    leader_task.abort();
    if leader::is_leader(&leader_state) {
        if let Err(e) = elector.relinquish().await {
            tracing::warn!(error = %e, "failed to relinquish lease on shutdown");
        }
    }
    tracing::info!("shutdown complete");

    Ok(())
}
