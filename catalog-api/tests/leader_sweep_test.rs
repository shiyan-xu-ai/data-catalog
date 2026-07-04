//! Two-"pod" leader-elected sweep integration test.
//!
//! Simplification (documented per plan): rather than spinning up two real OS processes,
//! this test runs two independent `sweep_loop` + `registry_cache` task pairs inside one
//! test process — one wired to a forced-on leader state, one to forced-off. That's exactly
//! the "forced leader on/off" testability path `catalog-api` exposes via
//! `CATALOG_LEADER_MODE`, so the loop code under test is identical to what runs in
//! production; only the *source* of the leadership boolean differs (a real k8s Lease vs. a
//! fixed value), and that boundary is `leader::LeaderElector`.
//!
//! To make "did the non-leader write?" observable rather than merely asserted, the leader
//! and non-leader sweep DIFFERENT roots containing DIFFERENT tables. If the non-leader's
//! sweep ever actually ran, its table would show up in the shared registry — it never does.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use catalog_store::SweepConfig;
use object_store::local::LocalFileSystem;
use object_store::path::Path as ObjPath;
use object_store::{ObjectStoreExt, PutPayload};
use tokio::sync::RwLock;

async fn write_fake_file(store: &LocalFileSystem, path: &str, content: &[u8]) {
    store
        .put(&ObjPath::from(path), PutPayload::from(content.to_vec()))
        .await
        .expect("write fake file");
}

fn sweep_config_over(tmp: &std::path::Path) -> SweepConfig {
    let store = Arc::new(LocalFileSystem::new_with_prefix(tmp).unwrap());
    SweepConfig::new(store, ObjPath::from(""), tmp.to_str().unwrap().to_string())
}

#[tokio::test]
async fn only_leader_writes_registry_and_both_pods_read_latest_state() {
    // Leader "pod": sweeps a root containing tableA.
    let leader_root = tempfile::tempdir().unwrap();
    let leader_store = LocalFileSystem::new_with_prefix(leader_root.path()).unwrap();
    write_fake_file(
        &leader_store,
        "tableA/2026-06-15-00-00-00/segments/_SUCCESS",
        b"",
    )
    .await;
    let leader_sweep_cfg = sweep_config_over(leader_root.path());

    // Non-leader "pod": sweeps a DIFFERENT root containing tableZ. If its sweep loop ever
    // actually runs (it shouldn't — it's gated on is_leader()), tableZ would leak into the
    // shared registry, which we assert against below.
    let follower_root = tempfile::tempdir().unwrap();
    let follower_store = LocalFileSystem::new_with_prefix(follower_root.path()).unwrap();
    write_fake_file(
        &follower_store,
        "tableZ/2026-06-15-00-00-00/segments/_SUCCESS",
        b"",
    )
    .await;
    let follower_sweep_cfg = sweep_config_over(follower_root.path());

    // Shared "_catalog/registry" location both pods write/read against.
    let registry_dir = tempfile::tempdir().unwrap();
    let registry_path = registry_dir
        .path()
        .join("registry.lance")
        .to_str()
        .unwrap()
        .to_string();

    let leader_state: Arc<AtomicBool> = Arc::new(AtomicBool::new(true));
    let follower_state: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
    // Each "pod" gets its own write_lock here -- they write to the same registry path but
    // this test never exercises the two-writer locking behavior (that's
    // `sweep_write_and_api_write_are_mutually_exclusive` in `sweep_loop.rs`); production
    // wiring in `main.rs` shares one lock across the sweep loop and the REST API.
    let leader_write_lock = catalog_api_lib::registry_lock::new_registry_write_lock();
    let follower_write_lock = catalog_api_lib::registry_lock::new_registry_write_lock();

    let tick = Duration::from_millis(30);
    let leader_sweep_handle = catalog_api_lib::sweep_loop::spawn_sweep_loop(
        leader_sweep_cfg,
        registry_path.clone(),
        leader_state,
        leader_write_lock,
        catalog_api_lib::sweep_loop::new_last_sweep_at(),
        tick,
    );
    let follower_sweep_handle = catalog_api_lib::sweep_loop::spawn_sweep_loop(
        follower_sweep_cfg,
        registry_path.clone(),
        follower_state,
        follower_write_lock,
        catalog_api_lib::sweep_loop::new_last_sweep_at(),
        tick,
    );

    // Both pods run a registry-refresh loop against the same shared registry path.
    let leader_cache = Arc::new(RwLock::new(Vec::new()));
    let follower_cache = Arc::new(RwLock::new(Vec::new()));
    let leader_refresh_handle = catalog_api_lib::registry_cache::spawn_refresh(
        registry_path.clone(),
        leader_cache.clone(),
        Duration::from_millis(20),
    );
    let follower_refresh_handle = catalog_api_lib::registry_cache::spawn_refresh(
        registry_path.clone(),
        follower_cache.clone(),
        Duration::from_millis(20),
    );

    // Give the leader's sweep loop several ticks to run, and the refresh loops several
    // ticks to pick up the write.
    tokio::time::sleep(Duration::from_millis(400)).await;

    leader_sweep_handle.abort();
    follower_sweep_handle.abort();
    leader_refresh_handle.abort();
    follower_refresh_handle.abort();

    // Only the leader wrote: tableA is in the registry, tableZ never made it in.
    let registry = catalog_core::read_registry(&registry_path)
        .await
        .expect("leader should have written the registry by now");
    assert_eq!(
        registry.len(),
        1,
        "only the leader's table should be present"
    );
    assert_eq!(registry[0].id, "tableA");
    assert!(registry.iter().all(|e| e.id != "tableZ"));

    // Both pods' read caches converge on the same leader-written state.
    let leader_seen = leader_cache.read().await;
    let follower_seen = follower_cache.read().await;
    assert_eq!(leader_seen.len(), 1);
    assert_eq!(leader_seen[0].id, "tableA");
    assert_eq!(follower_seen.len(), 1);
    assert_eq!(follower_seen[0].id, "tableA");
}
