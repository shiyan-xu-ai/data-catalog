use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use axum::extract::State;
use axum::{routing::get, Json, Router};
use tokio::sync::RwLock;
use tower_http::cors::CorsLayer;

use catalog_api_lib::api::{api_router, ApiState};
use catalog_api_lib::config::{AppConfig, LeaderMode};
use catalog_api_lib::leader::{
    self, ForcedLeaderElector, KubeLeaseElector, LeaderElector, LeaderState,
};
use catalog_api_lib::registry_cache::{self, RegistryCache};
use catalog_api_lib::registry_lock::{self, RegistryWriteLock};
use catalog_api_lib::sweep_config;
use catalog_api_lib::sweep_loop;
use catalog_store::SweepConfig;

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
        // Permissive CORS for the public /v1 + /ext + /metrics surface: v1.0.0 has no deployed
        // frontend yet, so there's no concrete origin to allow-list. TODO (Phase 9/10): tighten
        // to the frontend's actual deployed origin.
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

    // Shared with the sweep loop below: the SAME lock guards every writer's
    // read-registry -> mutate -> write-registry critical section (see `registry_lock`
    // module docs) so API mutations and the periodic sweep write can never interleave.
    let write_lock: RegistryWriteLock = registry_lock::new_registry_write_lock();

    let sweep_cfg = sweep_config::build_sweep_config(&cfg.sweep_root_uri)?;
    let _sweep_task = sweep_loop::spawn_sweep_loop(
        sweep_cfg.clone(),
        cfg.registry_path.clone(),
        leader_state.clone(),
        write_lock.clone(),
        cfg.sweep_interval,
    );

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
