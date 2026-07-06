//! `_catalog/storage_scan` Lance table: the derived bucket-storage breakdown produced by the
//! sync's storage-analysis tail step.
//!
//! Same JSON-into-Utf8-column pattern as `registry.rs`/`ttl_audit.rs`, but overwritten wholesale
//! on every scan (like the registry) rather than appended (like the audit log): a storage scan is
//! a point-in-time snapshot of the bucket layout, not a durable history, so each tail run replaces
//! the prior scan in full.

use std::sync::Arc;

use anyhow::{Context, Result};
use arrow_array::{
    Array, BooleanArray, RecordBatch, RecordBatchIterator, StringArray, UInt32Array, UInt64Array,
};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use lance::dataset::{WriteMode, WriteParams};
use lance::Dataset;

use crate::types::StoragePrefixStat;

fn storage_stats_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("region", DataType::Utf8, false),
        Field::new("bucket", DataType::Utf8, false),
        Field::new("prefix", DataType::Utf8, false),
        Field::new("registered", DataType::Boolean, false),
        Field::new("bytes", DataType::UInt64, true),
        Field::new("objects", DataType::UInt64, true),
        Field::new("table_count", DataType::UInt32, true),
        Field::new("scanned_at", DataType::Utf8, false),
    ]))
}

fn stats_to_batch(stats: &[StoragePrefixStat]) -> Result<RecordBatch> {
    let schema = storage_stats_schema();

    let region: StringArray = stats.iter().map(|s| Some(s.region.as_str())).collect();
    let bucket: StringArray = stats.iter().map(|s| Some(s.bucket.as_str())).collect();
    let prefix: StringArray = stats.iter().map(|s| Some(s.prefix.as_str())).collect();
    let registered: BooleanArray = stats.iter().map(|s| Some(s.registered)).collect();
    let bytes: UInt64Array = stats.iter().map(|s| s.bytes).collect();
    let objects: UInt64Array = stats.iter().map(|s| s.objects).collect();
    let table_count: UInt32Array = stats.iter().map(|s| s.table_count).collect();
    let scanned_at: StringArray = stats
        .iter()
        .map(|s| Some(s.scanned_at.to_rfc3339()))
        .collect();

    Ok(RecordBatch::try_new(
        schema,
        vec![
            Arc::new(region),
            Arc::new(bucket),
            Arc::new(prefix),
            Arc::new(registered),
            Arc::new(bytes),
            Arc::new(objects),
            Arc::new(table_count),
            Arc::new(scanned_at),
        ],
    )?)
}

fn batch_to_stats(batch: &RecordBatch) -> Result<Vec<StoragePrefixStat>> {
    let str_col = |name: &str| -> Result<&StringArray> {
        batch
            .column_by_name(name)
            .with_context(|| format!("missing column {name}"))?
            .as_any()
            .downcast_ref::<StringArray>()
            .with_context(|| format!("column {name} is not Utf8"))
    };
    let region = str_col("region")?;
    let bucket = str_col("bucket")?;
    let prefix = str_col("prefix")?;
    let registered = batch
        .column_by_name("registered")
        .context("missing column registered")?
        .as_any()
        .downcast_ref::<BooleanArray>()
        .context("column registered is not Boolean")?;
    let bytes = batch
        .column_by_name("bytes")
        .context("missing column bytes")?
        .as_any()
        .downcast_ref::<UInt64Array>()
        .context("column bytes is not UInt64")?;
    let objects = batch
        .column_by_name("objects")
        .context("missing column objects")?
        .as_any()
        .downcast_ref::<UInt64Array>()
        .context("column objects is not UInt64")?;
    let table_count = batch
        .column_by_name("table_count")
        .context("missing column table_count")?
        .as_any()
        .downcast_ref::<UInt32Array>()
        .context("column table_count is not UInt32")?;
    let scanned_at = str_col("scanned_at")?;

    let mut stats = Vec::with_capacity(batch.num_rows());
    for i in 0..batch.num_rows() {
        stats.push(StoragePrefixStat {
            region: region.value(i).to_string(),
            bucket: bucket.value(i).to_string(),
            prefix: prefix.value(i).to_string(),
            registered: registered.value(i),
            bytes: (!bytes.is_null(i)).then(|| bytes.value(i)),
            objects: (!objects.is_null(i)).then(|| objects.value(i)),
            table_count: (!table_count.is_null(i)).then(|| table_count.value(i)),
            scanned_at: chrono::DateTime::parse_from_rfc3339(scanned_at.value(i))
                .context("parse scanned_at")?
                .with_timezone(&chrono::Utc),
        });
    }
    Ok(stats)
}

/// Write `stats` to the Lance dataset at `path`, overwriting any existing scan there — a storage
/// scan is a point-in-time snapshot, not a durable history, so each tail run fully replaces it.
pub async fn write_storage_stats(path: &str, stats: &[StoragePrefixStat]) -> Result<()> {
    let schema = storage_stats_schema();
    let batch = stats_to_batch(stats)?;
    let reader = RecordBatchIterator::new(vec![Ok(batch)], schema);
    let params = WriteParams {
        mode: WriteMode::Overwrite,
        ..Default::default()
    };
    Dataset::write(reader, path, Some(params))
        .await
        .context("write storage_scan dataset")?;
    Ok(())
}

/// Best-effort prune of storage-scan Lance versions older than `older_than`, so the scan's
/// manifest history (one new version per tail-run `Overwrite`) does not grow without bound. A
/// no-op if the dataset doesn't exist yet. Errors are returned for the caller to log; the sync
/// treats a failed cleanup as non-fatal.
pub async fn cleanup_storage_stats(path: &str, older_than: chrono::Duration) -> Result<()> {
    let dataset = match Dataset::open(path).await {
        Ok(dataset) => dataset,
        Err(lance::Error::DatasetNotFound { .. }) => return Ok(()),
        Err(e) => return Err(e).context("open storage_scan dataset for cleanup"),
    };
    dataset
        .cleanup_old_versions(older_than, None, None)
        .await
        .context("cleanup old storage_scan versions")?;
    Ok(())
}

/// Read the current storage scan at `path`.
///
/// Returns `Ok(None)` only when the dataset does not exist yet (no scan has ever run). Every
/// other failure is propagated as `Err`.
pub async fn read_storage_stats(path: &str) -> Result<Option<Vec<StoragePrefixStat>>> {
    let dataset = match Dataset::open(path).await {
        Ok(dataset) => dataset,
        Err(lance::Error::DatasetNotFound { .. }) => return Ok(None),
        Err(e) => return Err(e).context("open storage_scan dataset"),
    };
    let batch = dataset
        .scan()
        .try_into_batch()
        .await
        .context("scan storage_scan dataset")?;
    Ok(Some(batch_to_stats(&batch)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn sample(prefix: &str, registered: bool, bytes: Option<u64>) -> StoragePrefixStat {
        StoragePrefixStat {
            region: "r1".to_string(),
            bucket: "b1".to_string(),
            prefix: prefix.to_string(),
            registered,
            bytes,
            objects: bytes.map(|_| 3),
            table_count: bytes.map(|_| 1),
            scanned_at: Utc.with_ymd_and_hms(2026, 7, 5, 0, 0, 0).unwrap(),
        }
    }

    #[tokio::test]
    async fn round_trips_registered_unexplored_and_root_rows_and_overwrite_replaces() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("storage_scan.lance");
        let path = path.to_str().unwrap();

        // Not created yet.
        assert!(read_storage_stats(path).await.unwrap().is_none());

        let first = vec![
            sample("ns1", true, Some(1024)),
            sample("junk", false, None),
            sample("(root)", false, None),
        ];
        write_storage_stats(path, &first).await.unwrap();

        let mut read_back = read_storage_stats(path).await.unwrap().unwrap();
        read_back.sort_by(|a, b| a.prefix.cmp(&b.prefix));
        let mut expected = first.clone();
        expected.sort_by(|a, b| a.prefix.cmp(&b.prefix));
        assert_eq!(read_back, expected);

        // A second write fully replaces the first (Overwrite, not append).
        let second = vec![sample("only_one_now", true, Some(2048))];
        write_storage_stats(path, &second).await.unwrap();
        let after_second = read_storage_stats(path).await.unwrap().unwrap();
        assert_eq!(after_second, second);
    }
}
