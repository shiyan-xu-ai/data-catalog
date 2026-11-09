//! Integration test for the registry Lance reader/writer round-trip. Lives in the integration
//! tier (not an inline unit module) because it does real Lance dataset IO through the crate's
//! public `write_registry`/`read_registry` API.

use catalog_core::{
    read_registry, write_registry, AuxEntry, AuxFormat, Namespace, TableEntry, TableVersion,
    VersionShape,
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
        category: None,
        dataset_path: None,
        row_count: None,
        schema_json: None,
        lance_version: None,
        writer_version: None,
    }];
    let version = TableVersion {
        version_id: "2026-06-26T12:00:00Z".to_string(),
        timestamp: ts,
        snapshot_path: format!("s3://bucket/{id}/2026-06-26_12-00-00"),
        shape: VersionShape::Full,
        partial: false,
        storage_bytes_total: 12288,
        lance_core_bytes: 8192,
        sidecar_bytes: 0,
        segments_bytes: 4096,
        other_aux_bytes: 0,
        row_count: Some(1000),
        num_fragments: Some(2),
        schema_json: Some(r#"{"fields":[]}"#.to_string()),
        num_indices: Some(1),
        lance_version: None,
        writer_version: None,
        aux: aux.clone(),
        synced_at: ts,
        object_count: Some(42),
    };
    TableEntry {
        id: id.to_string(),
        name: id.to_string(),
        namespace: Namespace::new(["scenario_dataset_export"]),
        root_location: format!("s3://bucket/{id}"),
        owner: Some("raymond".to_string()),
        region: "us-phoenix-1".into(),
        bucket: "bkt".into(),
        last_synced: Some(ts),
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

#[test]
fn older_table_json_ignores_retired_fields_without_losing_metadata() {
    let expected = sample_entry("legacy");
    let mut json = serde_json::to_value(&expected).unwrap();
    json["ttl_policy"] = serde_json::json!({"keep_last_n": 5});
    json["versions"][0]["protected"] = serde_json::json!(true);
    json["versions"][0]["deleting"] = serde_json::json!(true);

    let read_back: TableEntry = serde_json::from_value(json).unwrap();
    assert_eq!(read_back, expected);
    let written = serde_json::to_value(read_back).unwrap();
    assert!(written.get("ttl_policy").is_none());
    assert!(written["versions"][0].get("protected").is_none());
    assert!(written["versions"][0].get("deleting").is_none());
}

#[tokio::test]
async fn older_registry_column_and_version_json_remain_readable() {
    use std::sync::Arc;

    use arrow_array::{ArrayRef, RecordBatch, RecordBatchIterator, StringArray};
    use arrow_schema::{DataType, Field, Schema};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("registry.lance");
    let path = path.to_str().unwrap();
    let expected = sample_entry("legacy");
    let mut versions = serde_json::to_value(&expected.versions).unwrap();
    versions[0]["protected"] = serde_json::json!(true);
    let columns = [
        ("id", Some(expected.id.clone())),
        ("name", Some(expected.name.clone())),
        ("region", Some(expected.region.clone())),
        ("bucket", Some(expected.bucket.clone())),
        (
            "namespace_json",
            Some(serde_json::to_string(&expected.namespace).unwrap()),
        ),
        ("root_location", Some(expected.root_location.clone())),
        ("owner", expected.owner.clone()),
        (
            "ttl_policy_json",
            Some(r#"{"keep_last_n":5,"max_age_days":30}"#.into()),
        ),
        ("last_synced", expected.last_synced.map(|t| t.to_rfc3339())),
        (
            "versions_json",
            Some(serde_json::to_string(&versions).unwrap()),
        ),
        (
            "aux_latest_json",
            Some(serde_json::to_string(&expected.aux_latest).unwrap()),
        ),
    ];
    let schema = Arc::new(Schema::new(
        columns
            .iter()
            .map(|(name, _)| Field::new(*name, DataType::Utf8, true))
            .collect::<Vec<_>>(),
    ));
    let arrays: Vec<ArrayRef> = columns
        .into_iter()
        .map(|(_, value)| Arc::new(StringArray::from(vec![value])) as ArrayRef)
        .collect();
    let batch = RecordBatch::try_new(schema.clone(), arrays).unwrap();
    let reader = RecordBatchIterator::new(vec![Ok(batch)], schema);
    lance::Dataset::write(reader, path, None::<lance::dataset::WriteParams>)
        .await
        .unwrap();

    let read_back = read_registry(path).await.unwrap().unwrap();
    assert_eq!(read_back, vec![expected]);

    write_registry(path, &read_back).await.unwrap();
    let rewritten = lance::Dataset::open(path)
        .await
        .unwrap()
        .scan()
        .try_into_batch()
        .await
        .unwrap();
    assert!(rewritten.column_by_name("ttl_policy_json").is_none());
    assert_eq!(read_registry(path).await.unwrap().unwrap(), read_back);
}
