use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
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
use catalog_api_lib::registry_cache::{self, RegistryCache};
use catalog_api_lib::registry_lock::{self, RegistryWriteLock};
use catalog_api_lib::sweep_config;
use catalog_api_lib::sweep_loop;
use catalog_store::SweepConfig;

async fn healthz() -> &'static str {
    "ok"
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

/// Polls registry cache every 5 s and sets the `catalog_hydration_ready` gauge.
fn spawn_hydration_gauge_updater(cache: RegistryCache) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let ready = !cache.read().await.is_empty();
            metrics::set_hydration_ready(ready);
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    })
}

/// Polls `last_sweep_at` every 5 s and updates `catalog_snapshot_staleness_seconds`.
///
/// If a sweep has completed, reports `elapsed.as_secs_f64()` since the last successful write.
/// If no sweep has completed yet, reports time elapsed since the process started — the gauge
/// begins climbing from boot so `FreshnessBreach` fires if the first sweep never lands.
fn spawn_staleness_gauge_updater(
    last_sweep_at: catalog_api_lib::sweep_loop::LastSweepAt,
    process_start: std::time::Instant,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let staleness = {
                let guard = last_sweep_at.lock().await;
                match *guard {
                    Some(t) => t.elapsed().as_secs_f64(),
                    None => process_start.elapsed().as_secs_f64(),
                }
            };
            metrics::set_snapshot_staleness(staleness);
            tokio::time::sleep(Duration::from_secs(5)).await;
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

    // Register process metrics descriptors (RSS, CPU, open fds).
    metrics_process::Collector::default().describe();

    let cfg = AppConfig::from_env()?;

    let leader_state: LeaderState = Arc::new(AtomicBool::new(false));
    let elector = build_leader_elector(&cfg).await?;
    let _leader_task =
        leader::run_leader_election(elector, leader_state.clone(), cfg.leader_tick_interval);

    let registry_cache: RegistryCache = Arc::new(RwLock::new(Vec::new()));
    let _refresh_task = registry_cache::spawn_refresh(
        cfg.registry_path.clone(),
        registry_cache.clone(),
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
    let _hydration_gauge = spawn_hydration_gauge_updater(registry_cache.clone());
    let _staleness_gauge = spawn_staleness_gauge_updater(last_sweep_at, process_start);

    // Tokio runtime metrics collector.
    let runtime_monitor = tokio_metrics::RuntimeMonitor::new(&tokio::runtime::Handle::current());
    let _tokio_metrics = spawn_tokio_metrics_collector(runtime_monitor);

    // Internal server: /metrics + /healthz on a separate network-policy-restricted port.
    let _internal_task =
        internal_server::spawn_internal_server(cfg.metrics_bind_addr.clone(), app_metrics);

    let listener = tokio::net::TcpListener::bind(&cfg.bind_addr)
        .await
        .expect("failed to bind listener");
    axum::serve(
        listener,
        app(
            registry_cache,
            leader_state,
            cfg.registry_path.clone(),
            write_lock,
            sweep_cfg,
            cfg.ttl_audit_path.clone(),
        ),
    )
    .await
    .expect("server error");

    Ok(())
}
