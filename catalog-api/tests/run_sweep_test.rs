//! Integration tests for `run_sweep` (`catalog-api/src/sweep.rs`): the catalog is a curated
//! allowlist, so a sweep processes only the registered tables (those with an authored overlay),
//! never the whole root. Fixtures use `seg_only` version dirs (a `segments/` dir, no main lance)
//! so no real Lance dataset is needed — that keeps these tests about the registered-set logic,
//! not shape classification (which `catalog-store`'s sweep_test covers).

use std::sync::Arc;

use catalog_api_lib::sweep::run_sweep;
use catalog_store::MetaStore;
use object_store::local::LocalFileSystem;
use object_store::memory::InMemory;
use object_store::path::Path as ObjPath;
use object_store::{ObjectStoreExt, PutPayload};

mod common;

/// Write a minimal `seg_only` version (`<table>/<ts>/segments/{_SUCCESS,part}`) under the store.
async fn seed_version(store: &LocalFileSystem, table: &str, ts_dir: &str) {
    for name in ["_SUCCESS", "part-00000.snappy.parquet"] {
        store
            .put(
                &ObjPath::from(format!("{table}/{ts_dir}/segments/{name}")),
                PutPayload::from(b"x".to_vec()),
            )
            .await
            .expect("seed version file");
    }
}

/// Register a table by creating its (owner-less) authored overlay, as `DeclareTable` would.
async fn register(meta: &MetaStore, table: &str) {
    meta.mutate_meta(table, |_| {})
        .await
        .expect("register table");
}

fn version_ids(registry: &[catalog_core::TableEntry], id: &str) -> usize {
    registry
        .iter()
        .find(|e| e.id == id)
        .map(|e| e.versions.len())
        .unwrap_or(0)
}

#[tokio::test]
async fn sweep_processes_only_registered_tables() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalFileSystem::new_with_prefix(root.path()).unwrap();
    // Three tables exist on S3; only two of them get registered, and a third registered table
    // (`declared_no_data`) has no directory on S3 yet.
    seed_version(&store, "registered_a", "2026-01-01_00-00-00").await;
    seed_version(&store, "registered_a", "2026-01-02_00-00-00").await;
    seed_version(&store, "unregistered_b", "2026-01-01_00-00-00").await;

    let sweep_cfg = common::sweep_config(root.path());
    let meta = MetaStore::new(Arc::new(InMemory::new()), ObjPath::from("_catalog/meta"));
    register(&meta, "registered_a").await;
    register(&meta, "declared_no_data").await;

    let dir = tempfile::tempdir().unwrap();
    let registry_path = dir
        .path()
        .join("registry.lance")
        .to_str()
        .unwrap()
        .to_string();

    let report = run_sweep(&sweep_cfg, &registry_path, &meta).await.unwrap();
    // Only the two registered tables are checked; the unregistered one on S3 is never touched.
    assert_eq!(report.tables_checked, 2);
    assert_eq!(report.failed_tables, 0);

    let registry = catalog_core::read_registry(&registry_path)
        .await
        .unwrap()
        .unwrap();
    let mut ids: Vec<&str> = registry.iter().map(|e| e.id.as_str()).collect();
    ids.sort();
    assert_eq!(ids, vec!["declared_no_data", "registered_a"]);
    assert_eq!(version_ids(&registry, "registered_a"), 2);
    // A registered table with no S3 directory sweeps cleanly to zero versions (a stub).
    assert_eq!(version_ids(&registry, "declared_no_data"), 0);
}

#[tokio::test]
async fn deregistering_a_table_drops_it_from_the_snapshot_on_the_next_sweep() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalFileSystem::new_with_prefix(root.path()).unwrap();
    seed_version(&store, "t", "2026-01-01_00-00-00").await;

    let sweep_cfg = common::sweep_config(root.path());
    let meta = MetaStore::new(Arc::new(InMemory::new()), ObjPath::from("_catalog/meta"));
    register(&meta, "t").await;

    let dir = tempfile::tempdir().unwrap();
    let registry_path = dir
        .path()
        .join("registry.lance")
        .to_str()
        .unwrap()
        .to_string();

    // First sweep: registered, so it lands in the snapshot.
    run_sweep(&sweep_cfg, &registry_path, &meta).await.unwrap();
    let after_register = catalog_core::read_registry(&registry_path)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after_register.len(), 1);

    // Deregister (delete the overlay) — the S3 data is untouched — then sweep again.
    meta.delete_meta("t").await.unwrap();
    let report = run_sweep(&sweep_cfg, &registry_path, &meta).await.unwrap();
    assert_eq!(report.tables_checked, 0);

    // The allowlist shrank to empty, so the snapshot drops the table even though it is still on S3.
    let after_deregister = catalog_core::read_registry(&registry_path)
        .await
        .unwrap()
        .unwrap();
    assert!(after_deregister.is_empty());
}
