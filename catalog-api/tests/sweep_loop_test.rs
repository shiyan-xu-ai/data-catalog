//! Integration tests for the leader-only sweep loop: they do real Lance registry IO and (for
//! the mutual-exclusion regression) drive the real `declare_table` HTTP handler concurrently, so
//! they belong in the integration tier rather than an inline unit module. The pure `sweep_due`
//! gate is unit-tested inline in `sweep_loop.rs`.

mod common;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use catalog_api_lib::api::{api_router, ApiState};
use catalog_api_lib::leader::LeaderState;
use catalog_api_lib::registry_lock::new_registry_write_lock;
use catalog_api_lib::sweep_loop::{new_last_sweep_at, run_sweep_once, run_sweep_once_inner};
use catalog_core::{read_registry, write_registry, Namespace, TableEntry};
use tower::ServiceExt;

#[tokio::test]
async fn run_sweep_once_skips_the_write_if_leadership_is_already_lost() {
    let sweep_root = tempfile::tempdir().unwrap();
    let sweep_cfg = common::sweep_config(sweep_root.path());
    let registry_dir = tempfile::tempdir().unwrap();
    let registry_path = registry_dir
        .path()
        .join("registry.lance")
        .to_str()
        .unwrap()
        .to_string();
    let write_lock = new_registry_write_lock();
    let last_sweep_at = new_last_sweep_at();

    // Leadership already false by the time run_sweep_once is called -- simulates the
    // "lost leadership mid-sweep" window the re-check exists to close.
    let leader_state: LeaderState = Arc::new(AtomicBool::new(false));
    run_sweep_once(
        &sweep_cfg,
        &registry_path,
        &leader_state,
        &write_lock,
        &last_sweep_at,
    )
    .await
    .expect("sweep-merge should not fail even when the write is skipped");

    // No write ever happened: the registry dataset was never created.
    assert!(
        read_registry(&registry_path).await.unwrap().is_none(),
        "registry should not exist -- the write must have been skipped"
    );
    // last_sweep_at must remain None when the write was skipped.
    assert!(
        last_sweep_at.lock().await.is_none(),
        "last_sweep_at must stay None when registry write was skipped"
    );
}

#[tokio::test]
async fn run_sweep_once_writes_when_still_leader_at_write_time() {
    let sweep_root = tempfile::tempdir().unwrap();
    let sweep_cfg = common::sweep_config(sweep_root.path());
    let registry_dir = tempfile::tempdir().unwrap();
    let registry_path = registry_dir
        .path()
        .join("registry.lance")
        .to_str()
        .unwrap()
        .to_string();
    let write_lock = new_registry_write_lock();
    let last_sweep_at = new_last_sweep_at();

    let leader_state: LeaderState = Arc::new(AtomicBool::new(true));
    run_sweep_once(
        &sweep_cfg,
        &registry_path,
        &leader_state,
        &write_lock,
        &last_sweep_at,
    )
    .await
    .expect("sweep-merge-write should succeed");

    assert!(
        read_registry(&registry_path).await.unwrap().is_some(),
        "registry should have been written while still leader"
    );
    // last_sweep_at must be populated after a successful write.
    assert!(
        last_sweep_at.lock().await.is_some(),
        "last_sweep_at must be set after a successful sweep write"
    );
}

/// The sweep loop's read-modify-write and the API mutation handlers share the SAME
/// `RegistryWriteLock`, so their critical sections can never interleave. Before that fix the
/// sweep held no lock and could read the registry, then (after a concurrent API mutation
/// committed in between) write its own stale copy back on top, silently reverting the API write.
///
/// This drives the REAL `run_sweep_once_inner` (via its `after_read` pause seam) and the REAL
/// `declare_table` handler (via `api_router`) concurrently against the same registry path and
/// lock: the sweep reads a stale `owner: None`, then -- while still holding the lock -- pauses;
/// concurrently a `DeclareTable` call sets `owner: raymond` and must block on the same lock until
/// the sweep's write completes, so its write lands last and the owner survives.
#[tokio::test]
async fn sweep_write_and_api_write_are_mutually_exclusive() {
    let sweep_root = tempfile::tempdir().unwrap(); // empty: sweep finds no tables on S3
    let sweep_cfg = common::sweep_config(sweep_root.path());
    let registry_dir = tempfile::tempdir().unwrap();
    let registry_path = registry_dir
        .path()
        .join("registry.lance")
        .to_str()
        .unwrap()
        .to_string();

    let seed = vec![TableEntry {
        id: "smoke_test".to_string(),
        name: "smoke_test".to_string(),
        namespace: Namespace::new(["scenario_dataset_export"]),
        root_location: "s3://bucket/smoke_test".to_string(),
        owner: None,
        ttl_policy: None,
        last_swept: None,
        versions: Vec::new(),
        aux_latest: Vec::new(),
    }];
    write_registry(&registry_path, &seed).await.unwrap();

    let write_lock = new_registry_write_lock();
    let leader_state: LeaderState = Arc::new(AtomicBool::new(true));
    let after_read = Arc::new(tokio::sync::Notify::new());

    let last_sweep_at = new_last_sweep_at();
    let sweep_task = {
        let sweep_cfg = sweep_cfg.clone();
        let registry_path = registry_path.clone();
        let leader_state = leader_state.clone();
        let write_lock = write_lock.clone();
        let after_read = after_read.clone();
        let last_sweep_at = last_sweep_at.clone();
        tokio::spawn(async move {
            run_sweep_once_inner(
                &sweep_cfg,
                &registry_path,
                &leader_state,
                &write_lock,
                &last_sweep_at,
                Some(after_read),
            )
            .await
            .expect("sweep should not fail");
        })
    };

    // Wait until the sweep has read the (stale, owner=None) registry and is holding
    // write_lock through its artificial pause, before firing the racing API mutation.
    after_read.notified().await;

    let cache = Arc::new(tokio::sync::RwLock::new(seed));
    let state = ApiState::new(
        registry_path.clone(),
        cache,
        leader_state.clone(),
        write_lock.clone(),
        sweep_cfg.clone(),
        registry_dir
            .path()
            .join("ttl_audit.lance")
            .to_str()
            .unwrap()
            .to_string(),
    );
    let app = api_router(state);
    let declare_task = tokio::spawn(async move {
        app.oneshot(
            Request::builder()
                .method("PUT")
                .uri("/v1/table/smoke_test")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"owner":"raymond"}"#))
                .unwrap(),
        )
        .await
        .unwrap()
    });

    sweep_task.await.unwrap();
    let resp = declare_task.await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let final_registry = read_registry(&registry_path)
        .await
        .unwrap()
        .expect("registry exists after the sweep write");
    let entry = final_registry
        .iter()
        .find(|e| e.id == "smoke_test")
        .expect("table must still be present");
    assert_eq!(
        entry.owner.as_deref(),
        Some("raymond"),
        "the API-set owner must survive the racing sweep write -- the shared \
         RegistryWriteLock must prevent the sweep's stale read-then-write from clobbering it"
    );
}
