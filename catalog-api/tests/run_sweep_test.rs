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

/// The version-level carry-forward mechanic, end to end: clean versions are copied from the
/// prior snapshot without re-sweeping (immutable timestamp dirs — proven by an unchanged
/// `swept_at`), while partial versions, brand-new versions, and `deleting`-marked versions are
/// (re-)swept from S3.
#[tokio::test]
async fn carry_forward_skips_clean_versions_and_resweeps_partial_new_and_deleting() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalFileSystem::new_with_prefix(root.path()).unwrap();

    // v1: a REAL lance dataset dir (classifies LanceOnly, partial=false → carry-forwardable).
    // Reuse the registry writer to produce a genuine openable lance dataset without extra deps.
    let v1_lance = root
        .path()
        .join("t/2026-01-01_00-00-00/dataset.lance")
        .to_str()
        .unwrap()
        .to_string();
    catalog_core::write_registry(&v1_lance, &[]).await.unwrap();
    // v2: seg_only (classifies partial=true → never carried, re-swept every pass).
    seed_version(&store, "t", "2026-01-02_00-00-00").await;

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

    // Sweep #1: everything is new — both versions swept, nothing carried.
    let r1 = run_sweep(&sweep_cfg, &registry_path, &meta).await.unwrap();
    assert_eq!((r1.versions_swept, r1.versions_carried), (2, 0));
    let snap1 = catalog_core::read_registry(&registry_path)
        .await
        .unwrap()
        .unwrap();
    let v1_at = |snap: &[catalog_core::TableEntry], vid_prefix: &str| {
        snap.iter()
            .find(|e| e.id == "t")
            .unwrap()
            .versions
            .iter()
            .find(|v| v.version_id.starts_with(vid_prefix))
            .unwrap()
            .clone()
    };
    let v1_first = v1_at(&snap1, "2026-01-01");
    assert!(!v1_first.partial, "real lance dir must classify clean");
    assert!(v1_at(&snap1, "2026-01-02").partial, "seg_only is partial");

    // Sweep #2: nothing changed on S3 — the clean v1 is carried (identical swept_at, no S3
    // re-derive), the partial v2 is re-swept (self-heal path).
    let r2 = run_sweep(&sweep_cfg, &registry_path, &meta).await.unwrap();
    assert_eq!((r2.versions_swept, r2.versions_carried), (1, 1));
    let snap2 = catalog_core::read_registry(&registry_path)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        v1_at(&snap2, "2026-01-01").swept_at,
        v1_first.swept_at,
        "carried version keeps its original swept_at"
    );

    // Sweep #3: a new version lands — only it (plus the ever-partial v2) is swept.
    seed_version(&store, "t", "2026-01-03_00-00-00").await;
    let r3 = run_sweep(&sweep_cfg, &registry_path, &meta).await.unwrap();
    assert_eq!((r3.versions_swept, r3.versions_carried), (2, 1));

    // Sweep #4: mark v1 `deleting` (as a TTL apply would) — a deleting-marked version is never
    // carried, even though its prior classification is clean, so a partially-failed delete
    // can't freeze stale byte counts.
    let v1_id = v1_first.version_id.clone();
    meta.mutate_meta("t", |m| {
        m.deleting.insert(v1_id.clone());
    })
    .await
    .unwrap();
    let r4 = run_sweep(&sweep_cfg, &registry_path, &meta).await.unwrap();
    assert_eq!((r4.versions_swept, r4.versions_carried), (3, 0));
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
