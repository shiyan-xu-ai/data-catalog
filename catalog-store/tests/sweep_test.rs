//! Integration tests for the S3 sweep, run entirely against a local-filesystem
//! `object_store` (no real S3/MinIO — that's covered by a later phase's CI integration
//! test). Fixture trees are built by hand per the real layout documented in findings.md.

use std::sync::Arc;

use arrow_array::{Int32Array, RecordBatch, RecordBatchIterator};
use arrow_schema::{DataType, Field, Schema as ArrowSchema};
use catalog_core::{AuxFormat, VersionShape};
use catalog_store::SweepConfig;
use lance::Dataset;
use object_store::local::LocalFileSystem;
use object_store::path::Path as ObjPath;
use object_store::{ObjectStoreExt, PutPayload};

/// Write a small, real, openable lance dataset at the given filesystem path.
async fn write_lance_dataset(path: &std::path::Path) {
    let schema = Arc::new(ArrowSchema::new(vec![Field::new(
        "id",
        DataType::Int32,
        false,
    )]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(Int32Array::from(vec![1, 2, 3]))],
    )
    .unwrap();
    let reader = RecordBatchIterator::new(vec![Ok(batch)], schema);
    Dataset::write(
        reader,
        path.to_str().unwrap(),
        None::<lance::dataset::WriteParams>,
    )
    .await
    .expect("write lance dataset");
}

/// Write a fake (non-lance) file with the given content, creating parent dirs as needed.
async fn write_fake_file(store: &LocalFileSystem, path: &str, content: &[u8]) {
    store
        .put(&ObjPath::from(path), PutPayload::from(content.to_vec()))
        .await
        .expect("write fake file");
}

fn cfg(store: Arc<LocalFileSystem>, root_dir: &std::path::Path) -> SweepConfig {
    SweepConfig::new(
        store,
        ObjPath::from(""),
        root_dir.to_str().unwrap().to_string(),
    )
}

#[tokio::test]
async fn full_shape_post_cutoff() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(LocalFileSystem::new_with_prefix(tmp.path()).unwrap());
    let table_dir = tmp.path().join("tableA/2026-06-27_10-00-00");
    std::fs::create_dir_all(&table_dir).unwrap();

    write_lance_dataset(&table_dir.join("dataset.lance")).await;
    write_fake_file(
        &store,
        "tableA/2026-06-27_10-00-00/dataset.sidecar/manifest.bin",
        b"sidecar-content",
    )
    .await;
    write_fake_file(&store, "tableA/2026-06-27_10-00-00/segments/_SUCCESS", b"").await;
    write_fake_file(
        &store,
        "tableA/2026-06-27_10-00-00/segments/part-00000.snappy.parquet",
        b"fake-parquet-bytes",
    )
    .await;

    let cfg = cfg(store, tmp.path());
    let entry = catalog_store::sweep_table(&cfg, "tableA", &ObjPath::from("tableA"))
        .await
        .unwrap();

    assert_eq!(entry.versions.len(), 1);
    let v = &entry.versions[0];
    assert_eq!(v.shape, VersionShape::Full);
    assert!(!v.partial);
    assert_eq!(v.row_count, Some(3));
    assert_eq!(v.num_fragments, Some(1));
    assert!(v.schema_json.is_some());
    assert!(v.lance_core_bytes > 0, "lance_core_bytes should be > 0");
    assert!(v.sidecar_bytes > 0, "sidecar_bytes should be > 0");
    assert!(v.segments_bytes > 0, "segments_bytes should be > 0");
    assert_eq!(
        v.storage_bytes_total,
        v.lance_core_bytes + v.sidecar_bytes + v.segments_bytes + v.other_aux_bytes
    );

    let segments_aux = v.aux.iter().find(|a| a.name == "segments").unwrap();
    assert_eq!(segments_aux.format, AuxFormat::Parquet);
    let sidecar_aux = v.aux.iter().find(|a| a.name == "dataset.sidecar").unwrap();
    assert_eq!(sidecar_aux.storage_bytes, v.sidecar_bytes);
}

#[tokio::test]
async fn pre_cutoff_sidecar_inside_lance_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(LocalFileSystem::new_with_prefix(tmp.path()).unwrap());
    let table_dir = tmp.path().join("tableB/2026-06-20_09-00-00");
    std::fs::create_dir_all(&table_dir).unwrap();

    write_lance_dataset(&table_dir.join("dataset.lance")).await;
    // Pre-cutoff: sidecar dirs live *inside* the main lance dir, no top-level
    // dataset.sidecar/.
    write_fake_file(
        &store,
        "tableB/2026-06-20_09-00-00/dataset.lance/master_indices/idx.bin",
        b"master-indices-content",
    )
    .await;
    write_fake_file(
        &store,
        "tableB/2026-06-20_09-00-00/dataset.lance/lance_tags/tag.bin",
        b"lance-tags-content-longer",
    )
    .await;

    let cfg = cfg(store, tmp.path());
    let entry = catalog_store::sweep_table(&cfg, "tableB", &ObjPath::from("tableB"))
        .await
        .unwrap();

    assert_eq!(entry.versions.len(), 1);
    let v = &entry.versions[0];
    // No segments/ dir -> LanceOnly, not Full.
    assert_eq!(v.shape, VersionShape::LanceOnly);
    assert!(!v.partial);
    assert_eq!(v.row_count, Some(3));

    let expected_sidecar =
        b"master-indices-content".len() as u64 + b"lance-tags-content-longer".len() as u64;
    assert_eq!(v.sidecar_bytes, expected_sidecar);
    assert!(v.lance_core_bytes > 0);
    // lance-core bytes must not include the sidecar bytes we just attributed separately.
    assert_eq!(
        v.storage_bytes_total,
        v.lance_core_bytes + v.sidecar_bytes + v.segments_bytes + v.other_aux_bytes
    );

    assert!(v.aux.iter().any(|a| a.name == "master_indices"));
    assert!(v.aux.iter().any(|a| a.name == "lance_tags"));
}

#[tokio::test]
async fn seg_only_partial() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(LocalFileSystem::new_with_prefix(tmp.path()).unwrap());
    let table_dir = tmp.path().join("tableC/2026-06-15-00-00-00");
    std::fs::create_dir_all(&table_dir).unwrap();

    write_fake_file(&store, "tableC/2026-06-15-00-00-00/segments/_SUCCESS", b"").await;
    write_fake_file(
        &store,
        "tableC/2026-06-15-00-00-00/segments/part-00000.snappy.parquet",
        b"fake-parquet-bytes",
    )
    .await;

    let cfg = cfg(store, tmp.path());
    let entry = catalog_store::sweep_table(&cfg, "tableC", &ObjPath::from("tableC"))
        .await
        .unwrap();

    assert_eq!(entry.versions.len(), 1);
    let v = &entry.versions[0];
    assert_eq!(v.shape, VersionShape::SegOnly);
    assert!(v.partial);
    assert_eq!(v.row_count, None);
    assert_eq!(v.schema_json, None);
    assert_eq!(v.lance_core_bytes, 0);
    assert!(v.segments_bytes > 0);
}

#[tokio::test]
async fn transition_dedups_sidecar_bytes_but_keeps_both_aux_entries() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(LocalFileSystem::new_with_prefix(tmp.path()).unwrap());
    let table_dir = tmp.path().join("tableD/2026-06-12_00-00-00");
    std::fs::create_dir_all(&table_dir).unwrap();

    write_lance_dataset(&table_dir.join("dataset.lance")).await;
    // Dual-write: sidecar both inside dataset.lance/ AND at top-level dataset.sidecar/.
    write_fake_file(
        &store,
        "tableD/2026-06-12_00-00-00/dataset.lance/master_indices/idx.bin",
        b"inside-master-indices",
    )
    .await;
    write_fake_file(
        &store,
        "tableD/2026-06-12_00-00-00/dataset.lance/lance_tags/tag.bin",
        b"inside-lance-tags",
    )
    .await;
    write_fake_file(
        &store,
        "tableD/2026-06-12_00-00-00/dataset.sidecar/manifest.bin",
        b"top-level-sidecar-content",
    )
    .await;

    let cfg = cfg(store, tmp.path());
    let entry = catalog_store::sweep_table(&cfg, "tableD", &ObjPath::from("tableD"))
        .await
        .unwrap();

    assert_eq!(entry.versions.len(), 1);
    let v = &entry.versions[0];

    // Dedup: sidecar_bytes attributes ONLY the top-level dataset.sidecar/ content, not the
    // inside-lance sidecar dirs (would otherwise double-count the same logical sidecar).
    let top_level_len = b"top-level-sidecar-content".len() as u64;
    assert_eq!(v.sidecar_bytes, top_level_len);

    // But visibility is preserved: all three sidecar-related dirs still show up as aux
    // entries with their own real sizes, even though only one contributes to the
    // aggregate `sidecar_bytes` total.
    assert!(v.aux.iter().any(|a| a.name == "dataset.sidecar"));
    assert!(v.aux.iter().any(|a| a.name == "master_indices"));
    assert!(v.aux.iter().any(|a| a.name == "lance_tags"));

    assert_eq!(
        v.storage_bytes_total,
        v.lance_core_bytes + v.sidecar_bytes + v.segments_bytes + v.other_aux_bytes
    );
}

#[tokio::test]
async fn timestamp_dirname_separator_variance_parses_to_comparable_version_ids() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(LocalFileSystem::new_with_prefix(tmp.path()).unwrap());

    // Same table, two versions: one underscore-separated, one hyphen-separated.
    write_fake_file(&store, "tableE/2026-06-10_08-00-00/segments/_SUCCESS", b"").await;
    write_fake_file(&store, "tableE/2026-06-11-08-00-00/segments/_SUCCESS", b"").await;

    let cfg = cfg(store, tmp.path());
    let entry = catalog_store::sweep_table(&cfg, "tableE", &ObjPath::from("tableE"))
        .await
        .unwrap();

    assert_eq!(entry.versions.len(), 2);
    for v in &entry.versions {
        // Both dirname formats must normalize to a parseable RFC3339 version_id.
        chrono::DateTime::parse_from_rfc3339(&v.version_id)
            .unwrap_or_else(|e| panic!("version_id {} not RFC3339: {e}", v.version_id));
    }
    assert_ne!(entry.versions[0].version_id, entry.versions[1].version_id);
}

#[tokio::test]
async fn full_sweep_multiple_tables_multiple_versions() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(LocalFileSystem::new_with_prefix(tmp.path()).unwrap());

    // tableF: two versions, one Full (post-cutoff), one SegOnly (partial).
    let full_dir = tmp.path().join("tableF/2026-06-27_10-00-00");
    std::fs::create_dir_all(&full_dir).unwrap();
    write_lance_dataset(&full_dir.join("dataset.lance")).await;
    write_fake_file(&store, "tableF/2026-06-27_10-00-00/segments/_SUCCESS", b"").await;
    write_fake_file(&store, "tableF/2026-06-01_00-00-00/segments/_SUCCESS", b"").await;

    // tableG: one LanceOnly version.
    let lance_only_dir = tmp.path().join("tableG/2026-06-28_00-00-00");
    std::fs::create_dir_all(&lance_only_dir).unwrap();
    write_lance_dataset(&lance_only_dir.join("dataset.lance")).await;

    let cfg = cfg(store, tmp.path());
    let entries = catalog_store::sweep_root(&cfg).await.unwrap();

    assert_eq!(entries.len(), 2);
    let table_f = entries.iter().find(|e| e.id == "tableF").unwrap();
    let table_g = entries.iter().find(|e| e.id == "tableG").unwrap();

    assert_eq!(table_f.versions.len(), 2);
    let full_v = table_f
        .versions
        .iter()
        .find(|v| v.shape == VersionShape::Full)
        .expect("expected one Full version");
    assert!(!full_v.partial);
    let seg_only_v = table_f
        .versions
        .iter()
        .find(|v| v.shape == VersionShape::SegOnly)
        .expect("expected one SegOnly version");
    assert!(seg_only_v.partial);

    assert_eq!(table_g.versions.len(), 1);
    assert_eq!(table_g.versions[0].shape, VersionShape::LanceOnly);
    assert!(!table_g.versions[0].partial);
}
