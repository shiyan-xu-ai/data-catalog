//! Read-only DataFusion SQL over one registered table's current version.

use std::sync::Arc;
use std::time::Duration;

use arrow_json::writer::JsonArray;
use arrow_json::WriterBuilder;
use arrow_schema::{Field, Schema};
use lance::datafusion::LanceTableProvider;
use lance::datatypes::BlobHandling;
use lance::deps::arrow_array::RecordBatch;
use lance::deps::datafusion::common::TableReference;
use lance::deps::datafusion::error::DataFusionError;
use lance::deps::datafusion::execution::context::SQLOptions;
use lance::deps::datafusion::execution::disk_manager::{DiskManagerBuilder, DiskManagerMode};
use lance::deps::datafusion::execution::runtime_env::RuntimeEnvBuilder;
use lance::deps::datafusion::prelude::{SessionConfig, SessionContext};
use lance_namespace::{Error, Result};
use serde_json::Value;

use crate::namespace::{internal, invalid_input, split_table_id};
use crate::PostgresNamespace;

/// Bounds applied to every query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlLimits {
    /// Rows returned at most; a larger result is truncated and flagged.
    pub max_rows: usize,
    pub timeout: Duration,
    /// Memory pool of the query's DataFusion runtime; exceeding it fails the query.
    pub memory_limit_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlColumn {
    pub name: String,
    /// Arrow data type, as Arrow displays it (`Int32`, `Utf8`, ...).
    pub data_type: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SqlResult {
    pub columns: Vec<SqlColumn>,
    /// One JSON array per row, values encoded with Arrow's JSON writer.
    pub rows: Vec<Vec<Value>>,
    pub truncated: bool,
}

impl PostgresNamespace {
    /// Runs one read-only query against the table, registered under its leaf name.
    ///
    /// DDL, DML (including `COPY`), and other statements are rejected at planning; spilling is
    /// disabled so nothing touches the local disk; blob columns read as their descriptors, so
    /// payload bytes are never fetched.
    pub async fn query_sql(
        &self,
        id: &[String],
        sql: &str,
        limits: &SqlLimits,
    ) -> Result<SqlResult> {
        tokio::time::timeout(limits.timeout, self.run_sql(id, sql, limits))
            .await
            .map_err(|_| {
                invalid_input(format!(
                    "query exceeded the {}s time limit",
                    limits.timeout.as_secs()
                ))
            })?
    }

    async fn run_sql(&self, id: &[String], sql: &str, limits: &SqlLimits) -> Result<SqlResult> {
        let (_, name) = split_table_id(id)?;
        let (_, dataset) = self.open_table(id, None).await?;
        let provider = LanceTableProvider::new(Arc::new(dataset), false, false)
            .with_blob_handling(BlobHandling::BlobsDescriptions);
        let runtime = RuntimeEnvBuilder::new()
            .with_memory_limit(limits.memory_limit_bytes, 1.0)
            .with_disk_manager_builder(
                DiskManagerBuilder::default().with_mode(DiskManagerMode::Disabled),
            )
            .build_arc()
            .map_err(sql_error)?;
        let ctx = SessionContext::new_with_config_rt(SessionConfig::new(), runtime);
        ctx.register_table(TableReference::bare(name), Arc::new(provider))
            .map_err(sql_error)?;
        let options = SQLOptions::new()
            .with_allow_ddl(false)
            .with_allow_dml(false)
            .with_allow_statements(false);
        let frame = ctx
            .sql_with_options(sql, options)
            .await
            .map_err(sql_error)?;
        let schema = frame.schema().inner().clone();
        // One row past the limit tells whether the result was truncated.
        let batches = frame
            .limit(0, Some(limits.max_rows.saturating_add(1)))
            .map_err(sql_error)?
            .collect()
            .await
            .map_err(sql_error)?;
        let mut rows = encode_rows(&schema, &batches)?;
        let truncated = rows.len() > limits.max_rows;
        rows.truncate(limits.max_rows);
        Ok(SqlResult {
            columns: schema
                .fields()
                .iter()
                .map(|f| SqlColumn {
                    name: f.name().clone(),
                    data_type: f.data_type().to_string(),
                })
                .collect(),
            rows,
            truncated,
        })
    }
}

/// Encodes rows with Arrow's JSON writer. Columns are renamed positionally first so duplicate
/// output names (`SELECT 1, 1`) cannot collide in the writer's JSON objects.
fn encode_rows(schema: &Schema, batches: &[RecordBatch]) -> Result<Vec<Vec<Value>>> {
    let keys: Vec<String> = (0..schema.fields().len())
        .map(|i| format!("c{i}"))
        .collect();
    let positional = Arc::new(Schema::new(
        schema
            .fields()
            .iter()
            .zip(&keys)
            .map(|(f, key)| Field::new(key, f.data_type().clone(), true))
            .collect::<Vec<_>>(),
    ));
    let mut writer = WriterBuilder::new()
        .with_explicit_nulls(true)
        .build::<_, JsonArray>(Vec::new());
    for batch in batches {
        let batch = RecordBatch::try_new(positional.clone(), batch.columns().to_vec())
            .map_err(|e| internal(format!("encode query result: {e}")))?;
        writer
            .write(&batch)
            .map_err(|e| invalid_input(format!("cannot encode query result as JSON: {e}")))?;
    }
    writer
        .finish()
        .map_err(|e| internal(format!("encode query result: {e}")))?;
    let buffer = writer.into_inner();
    let objects: Vec<serde_json::Map<String, Value>> = if buffer.is_empty() {
        Vec::new()
    } else {
        serde_json::from_slice(&buffer)
            .map_err(|e| internal(format!("decode query result: {e}")))?
    };
    Ok(objects
        .into_iter()
        .map(|mut object| {
            keys.iter()
                .map(|key| object.remove(key).unwrap_or(Value::Null))
                .collect()
        })
        .collect())
}

/// Query mistakes and exceeded limits are invalid input carrying DataFusion's message; storage
/// and engine faults are internal.
fn sql_error(e: DataFusionError) -> Error {
    match e.find_root() {
        DataFusionError::ResourcesExhausted(message) => {
            invalid_input(format!("query exceeded the memory limit: {message}"))
        }
        DataFusionError::ObjectStore(_)
        | DataFusionError::IoError(_)
        | DataFusionError::External(_)
        | DataFusionError::Internal(_) => internal(format!("query failed: {e}")),
        _ => invalid_input(e.to_string()),
    }
}
