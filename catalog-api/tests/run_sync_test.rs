//! Integration tests for `run_sync` (`catalog-api/src/sync.rs`): the catalog is a curated
//! allowlist, so a sync processes only the registered tables (those with an authored overlay),
//! never the whole root. Fixtures use `seg_only` version dirs (a `segments/` dir, no main lance)
//! so no real Lance dataset is needed — that keeps these tests about the registered-set logic,
//! not shape classification (which `catalog-store`'s sync_test covers).

use std::sync::Arc;

use catalog_api_lib::sync::{run_sync, SyncTarget};
use catalog_core::compose_table_id;
use catalog_store::{MetaStore, SyncConfig};
use object_store::local::LocalFileSystem;
use object_store::memory::InMemory;
use object_store::path::Path as ObjPath;
use object_store::{ObjectStoreExt, PutPayload};

mod common;

/// Write a minimal `seg_only` version (`<dir>/<ts>/segments/{_SUCCESS,part}`) under the store.
/// `dir` is the version dir's parent path within the store (a namespace-qualified table dir in
/// the multi-namespace tests, or a bare table name in the single-target ones).
async fn seed_version(store: &LocalFileSystem, dir: &str, ts_dir: &str) {
    for name in ["_SUCCESS", "part-00000.snappy.parquet"] {
        store
            .put(
                &ObjPath::from(format!("{dir}/{ts_dir}/segments/{name}")),
                PutPayload::from(b"x".to_vec()),
            )
            .await
            .expect("seed version file");
    }
}

/// Register a table by creating its (owner-less) authored overlay, as `DeclareTable` would.
async fn register(meta: &MetaStore, id: &str) {
    meta.mutate_meta(id, |_| {}).await.expect("register table");
}

/// A single `SyncTarget` over one local-filesystem "bucket" rooted at `root`, scoped to
/// `namespace`. The store is rooted at `root`; `root_path` is the namespace prefix path and
/// `root_uri` is the local path equivalent of `s3://<bucket>/<ns>` so `uri_for` resolves the
/// same on-disk location. `namespace` empty ⇒ the store root is the namespace (bare table dirs).
fn local_target(
    root: &std::path::Path,
    region: &str,
    bucket: &str,
    namespace: &[&str],
) -> SyncTarget {
    let store = Arc::new(LocalFileSystem::new_with_prefix(root).unwrap());
    let ns: Vec<String> = namespace.iter().map(|s| s.to_string()).collect();
    let ns_path = ns.join("/");
    let root_path = ObjPath::from(ns_path.as_str());
    let root_uri = if ns_path.is_empty() {
        root.to_str().unwrap().to_string()
    } else {
        format!("{}/{ns_path}", root.to_str().unwrap())
    };
    let cfg = SyncConfig::new(store, root_path, root_uri);
    SyncTarget {
        region: region.to_string(),
        bucket: bucket.to_string(),
        namespace: ns,
        cfg,
    }
}

fn version_ids(registry: &[catalog_core::TableEntry], id: &str) -> usize {
    registry
        .iter()
        .find(|e| e.id == id)
        .map(|e| e.versions.len())
        .unwrap_or(0)
}

fn registry_path(dir: &tempfile::TempDir) -> String {
    dir.path()
        .join("registry.lance")
        .to_str()
        .unwrap()
        .to_string()
}

/// The architecturally central case: two buckets, two namespaces (one nested), synced into a
/// single registry snapshot. Each declared id is routed to the target whose
/// `(region, bucket, namespace)` matches, and the entry's `region`/`bucket`/`namespace` come
/// from that target — not from parsing the root path.
#[tokio::test]
async fn syncs_all_namespaces_across_buckets_into_one_snapshot() {
    let bucket_a = tempfile::tempdir().unwrap();
    let bucket_b = tempfile::tempdir().unwrap();
    let store_a = LocalFileSystem::new_with_prefix(bucket_a.path()).unwrap();
    let store_b = LocalFileSystem::new_with_prefix(bucket_b.path()).unwrap();

    // Bucket A / ns1 / t1 ; Bucket B / nsx/deep / t2. Version dirs live under their
    // namespace-qualified table dirs.
    seed_version(&store_a, "ns1/t1", "2026-01-01_00-00-00").await;
    seed_version(&store_b, "nsx/deep/t2", "2026-01-01_00-00-00").await;

    let t1 = compose_table_id("r1", "bA", &["ns1".into()], "t1");
    let t2 = compose_table_id("r1", "bB", &["nsx".into(), "deep".into()], "t2");
    assert_eq!(t1, "r1:bA:ns1:t1");
    assert_eq!(t2, "r1:bB:nsx:deep:t2");

    let meta = MetaStore::new(Arc::new(InMemory::new()), ObjPath::from("_catalog/meta"));
    register(&meta, &t1).await;
    register(&meta, &t2).await;

    let targets = vec![
        local_target(bucket_a.path(), "r1", "bA", &["ns1"]),
        local_target(bucket_b.path(), "r1", "bB", &["nsx", "deep"]),
    ];

    let dir = tempfile::tempdir().unwrap();
    let registry_path = registry_path(&dir);

    let report = run_sync(&targets, &registry_path, &meta).await.unwrap();
    assert_eq!(report.tables_checked, 2);
    assert_eq!(report.failed_tables, 0);

    let registry = catalog_core::read_registry(&registry_path)
        .await
        .unwrap()
        .unwrap();

    let mut ids: Vec<&str> = registry.iter().map(|e| e.id.as_str()).collect();
    ids.sort();
    assert_eq!(ids, vec![t1.as_str(), t2.as_str()]);

    let e1 = registry.iter().find(|e| e.id == t1).unwrap();
    assert_eq!(e1.name, "t1");
    assert_eq!(e1.region, "r1");
    assert_eq!(e1.bucket, "bA");
    assert_eq!(e1.namespace.segments(), &["ns1".to_string()]);
    assert_eq!(version_ids(&registry, &t1), 1);

    let e2 = registry.iter().find(|e| e.id == t2).unwrap();
    assert_eq!(e2.name, "t2");
    assert_eq!(e2.region, "r1");
    assert_eq!(e2.bucket, "bB");
    assert_eq!(
        e2.namespace.segments(),
        &["nsx".to_string(), "deep".to_string()]
    );
    assert_eq!(version_ids(&registry, &t2), 1);
}

#[tokio::test]
async fn sync_processes_only_registered_tables() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalFileSystem::new_with_prefix(root.path()).unwrap();
    // Three tables exist on S3 (under the `ns` namespace prefix); only two of them get
    // registered, and a third registered table (`declared_no_data`) has no directory on S3 yet.
    seed_version(&store, "ns/registered_a", "2026-01-01_00-00-00").await;
    seed_version(&store, "ns/registered_a", "2026-01-02_00-00-00").await;
    seed_version(&store, "ns/unregistered_b", "2026-01-01_00-00-00").await;

    let target = local_target(root.path(), "r1", "b1", &["ns"]);
    let reg_a = compose_table_id("r1", "b1", &["ns".into()], "registered_a");
    let no_data = compose_table_id("r1", "b1", &["ns".into()], "declared_no_data");
    let meta = MetaStore::new(Arc::new(InMemory::new()), ObjPath::from("_catalog/meta"));
    register(&meta, &reg_a).await;
    register(&meta, &no_data).await;

    let dir = tempfile::tempdir().unwrap();
    let registry_path = registry_path(&dir);

    let report = run_sync(&[target], &registry_path, &meta).await.unwrap();
    // Only the two registered tables are checked; the unregistered one on S3 is never touched.
    assert_eq!(report.tables_checked, 2);
    assert_eq!(report.failed_tables, 0);

    let registry = catalog_core::read_registry(&registry_path)
        .await
        .unwrap()
        .unwrap();
    let mut ids: Vec<&str> = registry.iter().map(|e| e.id.as_str()).collect();
    ids.sort();
    assert_eq!(ids, vec![no_data.as_str(), reg_a.as_str()]);
    assert_eq!(version_ids(&registry, &reg_a), 2);
    // A registered table with no S3 directory syncs cleanly to zero versions (a stub).
    assert_eq!(version_ids(&registry, &no_data), 0);
}

/// The version-level carry-forward mechanic, end to end: clean versions are copied from the
/// prior snapshot without re-syncing (immutable timestamp dirs — proven by an unchanged
/// `synced_at`), while partial versions, brand-new versions, and `deleting`-marked versions are
/// (re-)synced from S3.
#[tokio::test]
async fn carry_forward_skips_clean_versions_and_resyncs_partial_new_and_deleting() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalFileSystem::new_with_prefix(root.path()).unwrap();

    // v1: a REAL lance dataset dir (classifies LanceOnly, partial=false → carry-forwardable).
    // Reuse the registry writer to produce a genuine openable lance dataset without extra deps.
    let v1_lance = root
        .path()
        .join("ns/t/2026-01-01_00-00-00/dataset.lance")
        .to_str()
        .unwrap()
        .to_string();
    catalog_core::write_registry(&v1_lance, &[]).await.unwrap();
    // v2: seg_only (classifies partial=true → never carried, re-synced every pass).
    seed_version(&store, "ns/t", "2026-01-02_00-00-00").await;

    let target = local_target(root.path(), "r1", "b1", &["ns"]);
    let tid = compose_table_id("r1", "b1", &["ns".into()], "t");
    let meta = MetaStore::new(Arc::new(InMemory::new()), ObjPath::from("_catalog/meta"));
    register(&meta, &tid).await;

    let dir = tempfile::tempdir().unwrap();
    let registry_path = registry_path(&dir);

    // Sync #1: everything is new — both versions synced, nothing carried.
    let r1 = run_sync(std::slice::from_ref(&target), &registry_path, &meta)
        .await
        .unwrap();
    assert_eq!((r1.versions_synced, r1.versions_carried), (2, 0));
    let snap1 = catalog_core::read_registry(&registry_path)
        .await
        .unwrap()
        .unwrap();
    let v1_at = |snap: &[catalog_core::TableEntry], vid_prefix: &str| {
        snap.iter()
            .find(|e| e.id == tid)
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

    // Sync #2: nothing changed on S3 — the clean v1 is carried (identical synced_at, no S3
    // re-derive), the partial v2 is re-synced (self-heal path).
    let r2 = run_sync(std::slice::from_ref(&target), &registry_path, &meta)
        .await
        .unwrap();
    assert_eq!((r2.versions_synced, r2.versions_carried), (1, 1));
    let snap2 = catalog_core::read_registry(&registry_path)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        v1_at(&snap2, "2026-01-01").synced_at,
        v1_first.synced_at,
        "carried version keeps its original synced_at"
    );

    // Sync #3: a new version lands — only it (plus the ever-partial v2) is synced.
    seed_version(&store, "ns/t", "2026-01-03_00-00-00").await;
    let r3 = run_sync(std::slice::from_ref(&target), &registry_path, &meta)
        .await
        .unwrap();
    assert_eq!((r3.versions_synced, r3.versions_carried), (2, 1));

    // Sync #4: mark v1 `deleting` (as a TTL apply would) — a deleting-marked version is never
    // carried, even though its prior classification is clean, so a partially-failed delete
    // can't freeze stale byte counts.
    let v1_id = v1_first.version_id.clone();
    meta.mutate_meta(&tid, |m| {
        m.deleting.insert(v1_id.clone());
    })
    .await
    .unwrap();
    let r4 = run_sync(&[target], &registry_path, &meta).await.unwrap();
    assert_eq!((r4.versions_synced, r4.versions_carried), (3, 0));
}

#[tokio::test]
async fn deregistering_a_table_drops_it_from_the_snapshot_on_the_next_sync() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalFileSystem::new_with_prefix(root.path()).unwrap();
    seed_version(&store, "ns/t", "2026-01-01_00-00-00").await;

    let target = local_target(root.path(), "r1", "b1", &["ns"]);
    let tid = compose_table_id("r1", "b1", &["ns".into()], "t");
    let meta = MetaStore::new(Arc::new(InMemory::new()), ObjPath::from("_catalog/meta"));
    register(&meta, &tid).await;

    let dir = tempfile::tempdir().unwrap();
    let registry_path = registry_path(&dir);

    // First sync: registered, so it lands in the snapshot.
    run_sync(std::slice::from_ref(&target), &registry_path, &meta)
        .await
        .unwrap();
    let after_register = catalog_core::read_registry(&registry_path)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after_register.len(), 1);

    // Deregister (delete the overlay) — the S3 data is untouched — then sync again.
    meta.delete_meta(&tid).await.unwrap();
    let report = run_sync(&[target], &registry_path, &meta).await.unwrap();
    assert_eq!(report.tables_checked, 0);

    // The allowlist shrank to empty, so the snapshot drops the table even though it is still on S3.
    let after_deregister = catalog_core::read_registry(&registry_path)
        .await
        .unwrap()
        .unwrap();
    assert!(after_deregister.is_empty());
}
