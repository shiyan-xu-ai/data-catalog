//! `_catalog/ttl_audit` Lance table: append-only record of every TTL hard-delete.
//!
//! Same JSON-into-Utf8-column pattern as `registry.rs`. "Append" is implemented as
//! read-existing + concatenate + full `WriteMode::Overwrite` rewrite (not Lance's own
//! `WriteMode::Append`), reusing the exact write path `registry.rs` already proves correct
//! with its round-trip test. This is fine at v1.0.0's audit-log scale: one write per TTL
//! `apply` call (admin-triggered, not a hot path), and all TTL applies happen on the leader
//! under the same `RegistryWriteLock`-guarded critical section as the registry write, so
//! there is no concurrent-writer hazard on this path either.

use std::sync::Arc;

use anyhow::{Context, Result};
use arrow_array::{Array, RecordBatch, RecordBatchIterator, StringArray, UInt64Array};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use lance::dataset::{WriteMode, WriteParams};
use lance::Dataset;

use crate::types::{TtlAuditRecord, TtlPolicy};

fn ttl_audit_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("table_id", DataType::Utf8, false),
        Field::new("version_id", DataType::Utf8, false),
        Field::new("deleted_at", DataType::Utf8, false),
        Field::new("reclaimed_bytes", DataType::UInt64, false),
        Field::new("policy_snapshot_json", DataType::Utf8, false),
        Field::new("actor", DataType::Utf8, false),
    ]))
}

fn records_to_batch(records: &[TtlAuditRecord]) -> Result<RecordBatch> {
    let schema = ttl_audit_schema();

    let table_id: StringArray = records.iter().map(|r| Some(r.table_id.as_str())).collect();
    let version_id: StringArray = records
        .iter()
        .map(|r| Some(r.version_id.as_str()))
        .collect();
    let deleted_at: StringArray = records
        .iter()
        .map(|r| Some(r.deleted_at.to_rfc3339()))
        .collect();
    let reclaimed_bytes: UInt64Array = records.iter().map(|r| Some(r.reclaimed_bytes)).collect();
    let policy_snapshot_json: StringArray = records
        .iter()
        .map(|r| serde_json::to_string(&r.policy_snapshot))
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("serialize policy_snapshot")?
        .into_iter()
        .map(Some)
        .collect();
    let actor: StringArray = records.iter().map(|r| Some(r.actor.as_str())).collect();

    Ok(RecordBatch::try_new(
        schema,
        vec![
            Arc::new(table_id),
            Arc::new(version_id),
            Arc::new(deleted_at),
            Arc::new(reclaimed_bytes),
            Arc::new(policy_snapshot_json),
            Arc::new(actor),
        ],
    )?)
}

fn batch_to_records(batch: &RecordBatch) -> Result<Vec<TtlAuditRecord>> {
    let str_col = |name: &str| -> Result<&StringArray> {
        batch
            .column_by_name(name)
            .with_context(|| format!("missing column {name}"))?
            .as_any()
            .downcast_ref::<StringArray>()
            .with_context(|| format!("column {name} is not Utf8"))
    };
    let table_id = str_col("table_id")?;
    let version_id = str_col("version_id")?;
    let deleted_at = str_col("deleted_at")?;
    let reclaimed_bytes = batch
        .column_by_name("reclaimed_bytes")
        .context("missing column reclaimed_bytes")?
        .as_any()
        .downcast_ref::<UInt64Array>()
        .context("column reclaimed_bytes is not UInt64")?;
    let policy_snapshot_json = str_col("policy_snapshot_json")?;
    let actor = str_col("actor")?;

    let mut records = Vec::with_capacity(batch.num_rows());
    for i in 0..batch.num_rows() {
        let policy_snapshot: TtlPolicy = serde_json::from_str(policy_snapshot_json.value(i))
            .context("deserialize policy_snapshot")?;
        records.push(TtlAuditRecord {
            table_id: table_id.value(i).to_string(),
            version_id: version_id.value(i).to_string(),
            deleted_at: chrono::DateTime::parse_from_rfc3339(deleted_at.value(i))
                .context("parse deleted_at")?
                .with_timezone(&chrono::Utc),
            reclaimed_bytes: reclaimed_bytes.value(i),
            policy_snapshot,
            actor: actor.value(i).to_string(),
        });
    }
    Ok(records)
}

/// Read all audit records at `path`. Returns an empty vec if the dataset doesn't exist yet
/// (no TTL deletion has ever happened for any table).
pub async fn read_ttl_audit(path: &str) -> Result<Vec<TtlAuditRecord>> {
    match Dataset::open(path).await {
        Ok(dataset) => {
            let batch = dataset
                .scan()
                .try_into_batch()
                .await
                .context("scan ttl_audit dataset")?;
            batch_to_records(&batch)
        }
        Err(_) => Ok(Vec::new()),
    }
}

/// Append `new_records` to the audit log at `path`. A no-op if `new_records` is empty --
/// never creates an (empty) dataset just to record that nothing happened.
pub async fn append_ttl_audit(path: &str, new_records: &[TtlAuditRecord]) -> Result<()> {
    if new_records.is_empty() {
        return Ok(());
    }
    let mut all = read_ttl_audit(path).await?;
    all.extend(new_records.iter().cloned());

    let schema = ttl_audit_schema();
    let batch = records_to_batch(&all)?;
    let reader = RecordBatchIterator::new(vec![Ok(batch)], schema);
    let params = WriteParams {
        mode: WriteMode::Overwrite,
        ..Default::default()
    };
    Dataset::write(reader, path, Some(params))
        .await
        .context("write ttl_audit dataset")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TtlPolicy;
    use chrono::{TimeZone, Utc};

    fn sample(table_id: &str, version_id: &str) -> TtlAuditRecord {
        TtlAuditRecord {
            table_id: table_id.to_string(),
            version_id: version_id.to_string(),
            deleted_at: Utc.with_ymd_and_hms(2026, 7, 3, 0, 0, 0).unwrap(),
            reclaimed_bytes: 4096,
            policy_snapshot: TtlPolicy {
                keep_last_n: Some(5),
                max_age_days: Some(30),
            },
            actor: "ttl-engine".to_string(),
        }
    }

    #[tokio::test]
    async fn append_accumulates_across_calls_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ttl_audit.lance");
        let path = path.to_str().unwrap();

        assert!(read_ttl_audit(path).await.unwrap().is_empty());

        append_ttl_audit(path, &[sample("t1", "v1")]).await.unwrap();
        append_ttl_audit(path, &[sample("t1", "v2"), sample("t2", "v1")])
            .await
            .unwrap();

        let mut records = read_ttl_audit(path).await.unwrap();
        records.sort_by(|a, b| {
            (a.table_id.as_str(), a.version_id.as_str())
                .cmp(&(b.table_id.as_str(), b.version_id.as_str()))
        });
        assert_eq!(records.len(), 3);
        assert_eq!(records[0].table_id, "t1");
        assert_eq!(records[0].version_id, "v1");
        assert_eq!(records[1].version_id, "v2");
        assert_eq!(records[2].table_id, "t2");
    }

    #[tokio::test]
    async fn append_with_empty_slice_is_a_no_op_and_does_not_create_a_dataset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ttl_audit.lance");
        let path = path.to_str().unwrap();

        append_ttl_audit(path, &[]).await.unwrap();
        assert!(read_ttl_audit(path).await.unwrap().is_empty());
        assert!(
            Dataset::open(path).await.is_err(),
            "no dataset should have been created for an empty append"
        );
    }
}
