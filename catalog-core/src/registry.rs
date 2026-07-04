//! Registry Lance table: writer/reader for `TableEntry` rows.
//!
//! Nested fields (`namespace`, `ttl_policy`, `versions`, `aux_latest`) are JSON-encoded
//! into `Utf8` columns rather than modeled as deep Arrow structs — kept minimal for this
//! phase per the v1.0.0 plan.

use std::sync::Arc;

use anyhow::{Context, Result};
use arrow_array::{Array, RecordBatch, RecordBatchIterator, StringArray};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use lance::dataset::{WriteMode, WriteParams};
use lance::Dataset;

use crate::types::{Namespace, TableEntry, TtlPolicy};

fn registry_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("name", DataType::Utf8, false),
        Field::new("namespace_json", DataType::Utf8, false),
        Field::new("root_location", DataType::Utf8, false),
        Field::new("owner", DataType::Utf8, true),
        Field::new("ttl_policy_json", DataType::Utf8, true),
        Field::new("last_swept", DataType::Utf8, true),
        Field::new("versions_json", DataType::Utf8, false),
        Field::new("aux_latest_json", DataType::Utf8, false),
    ]))
}

fn entries_to_batch(entries: &[TableEntry]) -> Result<RecordBatch> {
    let schema = registry_schema();

    let id: StringArray = entries.iter().map(|e| Some(e.id.as_str())).collect();
    let name: StringArray = entries.iter().map(|e| Some(e.name.as_str())).collect();
    let namespace_json: StringArray = entries
        .iter()
        .map(|e| serde_json::to_string(&e.namespace))
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("serialize namespace")?
        .into_iter()
        .map(Some)
        .collect();
    let root_location: StringArray = entries
        .iter()
        .map(|e| Some(e.root_location.as_str()))
        .collect();
    let owner: StringArray = entries.iter().map(|e| e.owner.as_deref()).collect();
    let ttl_policy_json: StringArray = entries
        .iter()
        .map(|e| e.ttl_policy.as_ref().map(serde_json::to_string).transpose())
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("serialize ttl_policy")?
        .into_iter()
        .collect();
    let last_swept: StringArray = entries
        .iter()
        .map(|e| e.last_swept.map(|ts| ts.to_rfc3339()))
        .collect();
    let versions_json: StringArray = entries
        .iter()
        .map(|e| serde_json::to_string(&e.versions))
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("serialize versions")?
        .into_iter()
        .map(Some)
        .collect();
    let aux_latest_json: StringArray = entries
        .iter()
        .map(|e| serde_json::to_string(&e.aux_latest))
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("serialize aux_latest")?
        .into_iter()
        .map(Some)
        .collect();

    Ok(RecordBatch::try_new(
        schema,
        vec![
            Arc::new(id),
            Arc::new(name),
            Arc::new(namespace_json),
            Arc::new(root_location),
            Arc::new(owner),
            Arc::new(ttl_policy_json),
            Arc::new(last_swept),
            Arc::new(versions_json),
            Arc::new(aux_latest_json),
        ],
    )?)
}

fn batch_to_entries(batch: &RecordBatch) -> Result<Vec<TableEntry>> {
    let col = |name: &str| -> Result<&StringArray> {
        batch
            .column_by_name(name)
            .with_context(|| format!("missing column {name}"))?
            .as_any()
            .downcast_ref::<StringArray>()
            .with_context(|| format!("column {name} is not Utf8"))
    };

    let id = col("id")?;
    let name = col("name")?;
    let namespace_json = col("namespace_json")?;
    let root_location = col("root_location")?;
    let owner = col("owner")?;
    let ttl_policy_json = col("ttl_policy_json")?;
    let last_swept = col("last_swept")?;
    let versions_json = col("versions_json")?;
    let aux_latest_json = col("aux_latest_json")?;

    let mut entries = Vec::with_capacity(batch.num_rows());
    for i in 0..batch.num_rows() {
        let namespace: Namespace =
            serde_json::from_str(namespace_json.value(i)).context("deserialize namespace")?;
        let ttl_policy: Option<TtlPolicy> = if ttl_policy_json.is_null(i) {
            None
        } else {
            Some(serde_json::from_str(ttl_policy_json.value(i)).context("deserialize ttl_policy")?)
        };
        let last_swept = if last_swept.is_null(i) {
            None
        } else {
            Some(
                chrono::DateTime::parse_from_rfc3339(last_swept.value(i))
                    .context("parse last_swept")?
                    .with_timezone(&chrono::Utc),
            )
        };
        let versions =
            serde_json::from_str(versions_json.value(i)).context("deserialize versions")?;
        let aux_latest =
            serde_json::from_str(aux_latest_json.value(i)).context("deserialize aux_latest")?;

        entries.push(TableEntry {
            id: id.value(i).to_string(),
            name: name.value(i).to_string(),
            namespace,
            root_location: root_location.value(i).to_string(),
            owner: if owner.is_null(i) {
                None
            } else {
                Some(owner.value(i).to_string())
            },
            ttl_policy,
            last_swept,
            versions,
            aux_latest,
        });
    }
    Ok(entries)
}

/// Write the given registry entries to a Lance dataset at `path`, overwriting any existing
/// dataset there.
pub async fn write_registry(path: &str, entries: &[TableEntry]) -> Result<()> {
    let schema = registry_schema();
    let batch = entries_to_batch(entries)?;
    let reader = RecordBatchIterator::new(vec![Ok(batch)], schema);
    let params = WriteParams {
        mode: WriteMode::Overwrite,
        ..Default::default()
    };
    Dataset::write(reader, path, Some(params))
        .await
        .context("write registry dataset")?;
    Ok(())
}

/// Read all registry entries from the Lance dataset at `path`.
pub async fn read_registry(path: &str) -> Result<Vec<TableEntry>> {
    let dataset = Dataset::open(path).await.context("open registry dataset")?;
    let batch = dataset
        .scan()
        .try_into_batch()
        .await
        .context("scan registry dataset")?;
    batch_to_entries(&batch)
}
