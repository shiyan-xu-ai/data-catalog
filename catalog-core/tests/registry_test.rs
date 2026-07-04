//! Integration test for the registry Lance reader/writer round-trip. Lives in the integration
//! tier (not an inline unit module) because it does real Lance dataset IO through the crate's
//! public `write_registry`/`read_registry` API.

use catalog_core::{
    read_registry, write_registry, AuxEntry, AuxFormat, Namespace, TableEntry, TableVersion,
    TtlPolicy, VersionShape,
};
use chrono::{TimeZone, Utc};

fn sample_entry(id: &str) -> TableEntry {
    let ts = Utc.with_ymd_and_hms(2026, 6, 26, 12, 0, 0).unwrap();
    let aux = vec![AuxEntry {
        name: "segments".to_string(),
        path: format!("s3://bucket/{id}/2026-06-26_12-00-00/segments"),
        format: AuxFormat::Parquet,
        role: "segments".to_string(),
        storage_bytes: 4096,
        fingerprint: None,
    }];
    let version = TableVersion {
        version_id: "2026-06-26T12:00:00Z".to_string(),
        timestamp: ts,
        snapshot_path: format!("s3://bucket/{id}/2026-06-26_12-00-00"),
        shape: VersionShape::Full,
        partial: false,
        protected: false,
        storage_bytes_total: 12288,
        lance_core_bytes: 8192,
        sidecar_bytes: 0,
        segments_bytes: 4096,
        other_aux_bytes: 0,
        row_count: Some(1000),
        num_fragments: Some(2),
        schema_json: Some(r#"{"fields":[]}"#.to_string()),
        num_indices: Some(1),
        aux: aux.clone(),
        swept_at: ts,
    };
    TableEntry {
        id: id.to_string(),
        name: id.to_string(),
        namespace: Namespace::new(["scenario_dataset_export"]),
        root_location: format!("s3://bucket/{id}"),
        owner: Some("raymond".to_string()),
        ttl_policy: Some(TtlPolicy {
            keep_last_n: Some(5),
            max_age_days: Some(30),
        }),
        last_swept: Some(ts),
        versions: vec![version],
        aux_latest: aux,
    }
}

#[tokio::test]
async fn registry_round_trips_table_entries() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("registry.lance");
    let path = path.to_str().unwrap();

    let entries = vec![sample_entry("smoke_test"), sample_entry("closed_loop")];

    write_registry(path, &entries).await.unwrap();
    let mut read_back = read_registry(path)
        .await
        .unwrap()
        .expect("registry exists after write");
    read_back.sort_by(|a, b| a.id.cmp(&b.id));

    let mut expected = entries;
    expected.sort_by(|a, b| a.id.cmp(&b.id));

    assert_eq!(read_back, expected);
}
