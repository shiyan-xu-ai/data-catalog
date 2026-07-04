use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use axum::extract::State;
use axum::{routing::get, Json, Router};
use tokio::sync::RwLock;

use catalog_api_lib::config::{AppConfig, LeaderMode};
use catalog_api_lib::leader::{
    self, ForcedLeaderElector, KubeLeaseElector, LeaderElector, LeaderState,
};
use catalog_api_lib::registry_cache::{self, RegistryCache};
use catalog_api_lib::sweep_config;
use catalog_api_lib::sweep_loop;

async fn healthz() -> &'static str {
    "ok"
}

/// Internal debug endpoint: inspect the currently cached registry state. Not part of the
/// public REST API (that's a later phase) — useful for local dev and integration tests.
async fn debug_registry(State(cache): State<RegistryCache>) -> Json<Vec<catalog_core::TableEntry>> {
    Json(cache.read().await.clone())
}

async fn debug_is_leader(State(state): State<LeaderState>) -> Json<bool> {
    Json(leader::is_leader(&state))
}

fn app(registry_cache: RegistryCache, leader_state: LeaderState) -> Router {
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

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

    let sweep_cfg = sweep_config::build_sweep_config(&cfg.sweep_root_uri)?;
    let _sweep_task = sweep_loop::spawn_sweep_loop(
        sweep_cfg,
        cfg.registry_path.clone(),
        leader_state.clone(),
        cfg.sweep_interval,
    );

    let listener = tokio::net::TcpListener::bind(&cfg.bind_addr)
        .await
        .expect("failed to bind listener");
    axum::serve(listener, app(registry_cache, leader_state))
        .await
        .expect("server error");

    Ok(())
}
