use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use arrow_json::writer::JsonArray;
use arrow_json::WriterBuilder;
use arrow_schema::{Field, Schema};
use async_trait::async_trait;
use lance::datafusion::LanceTableProvider;
use lance::datatypes::BlobHandling;
use lance::deps::arrow_array::RecordBatch;
use lance::deps::datafusion::catalog::{
    CatalogProvider, MemoryCatalogProvider, SchemaProvider, TableProvider,
};
use lance::deps::datafusion::error::DataFusionError;
use lance::deps::datafusion::execution::context::SQLOptions;
use lance::deps::datafusion::execution::disk_manager::{DiskManagerBuilder, DiskManagerMode};
use lance::deps::datafusion::execution::runtime_env::RuntimeEnvBuilder;
use lance::deps::datafusion::prelude::{SessionConfig, SessionContext};
use lance::Dataset;
use lance_namespace::{Error, NamespaceError, Result};
use serde_json::Value;

use crate::namespace::{internal, invalid_input, is_missing, split_table_id, OpenTable};
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
    /// Version the plain table name resolved to.
    pub version: u64,
    pub columns: Vec<SqlColumn>,
    /// One JSON array per row, values encoded with Arrow's JSON writer.
    pub rows: Vec<Vec<Value>>,
    pub truncated: bool,
}

impl PostgresNamespace {
    /// Runs one read-only query against the table. Its leaf name resolves to `version` (the
    /// current version when unset); `"<leaf>@v<N>"` resolves to version N, opened on demand.
    ///
    /// DDL, DML (including `COPY`), and other statements are rejected at planning; spilling is
    /// disabled so nothing touches the local disk; blob columns read as their descriptors, so
    /// payload bytes are never fetched. The time limit covers opening every version.
    pub async fn query_sql(
        &self,
        id: &[String],
        sql: &str,
        version: Option<u64>,
        limits: &SqlLimits,
    ) -> Result<SqlResult> {
        if version == Some(0) {
            return Err(invalid_input("version must be a positive integer".into()));
        }
        tokio::time::timeout(limits.timeout, self.run_sql(id, sql, version, limits))
            .await
            .map_err(|_| {
                invalid_input(format!(
                    "query exceeded the {}s time limit",
                    limits.timeout.as_secs()
                ))
            })?
    }

    async fn run_sql(
        &self,
        id: &[String],
        sql: &str,
        version: Option<u64>,
        limits: &SqlLimits,
    ) -> Result<SqlResult> {
        let (_, name) = split_table_id(id)?;
        self.read_table(id, version, |table| run_on(table, name, sql, limits))
            .await
    }
}

async fn run_on(table: OpenTable, name: &str, sql: &str, limits: &SqlLimits) -> Result<SqlResult> {
    let OpenTable {
        location, dataset, ..
    } = table;
    let version = dataset.version_id();
    let runtime = RuntimeEnvBuilder::new()
        .with_memory_limit(limits.memory_limit_bytes, 1.0)
        .with_disk_manager_builder(
            DiskManagerBuilder::default().with_mode(DiskManagerMode::Disabled),
        )
        .build_arc()
        .map_err(sql_error)?;
    let config = SessionConfig::new();
    let defaults = &config.options().catalog;
    let (catalog_name, schema_name) = (
        defaults.default_catalog.clone(),
        defaults.default_schema.clone(),
    );
    let ctx = SessionContext::new_with_config_rt(config, runtime);
    let schema = Arc::new(VersionedTable::new(name, location, dataset));
    let catalog = MemoryCatalogProvider::new();
    catalog
        .register_schema(&schema_name, schema.clone())
        .map_err(sql_error)?;
    ctx.register_catalog(catalog_name, Arc::new(catalog));
    let options = SQLOptions::new()
        .with_allow_ddl(false)
        .with_allow_dml(false)
        .with_allow_statements(false);
    let frame = match ctx.sql_with_options(sql, options).await {
        Ok(frame) => frame,
        // A failed version open surfaces as a planning error; report its own error.
        Err(e) => return Err(schema.take_open_error().unwrap_or_else(|| sql_error(e))),
    };
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
        version,
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

/// The only schema of a query's session: the table's leaf name at the requested version, plus
/// `"<leaf>@v<N>"` names that open version N on first use. Every other name is not found.
struct VersionedTable {
    name: String,
    location: String,
    dataset: Arc<Dataset>,
    versions: Mutex<HashMap<u64, Arc<dyn TableProvider>>>,
    /// The error of the first version that failed to open, which DataFusion would otherwise
    /// flatten into a planning message.
    open_error: Mutex<Option<Error>>,
}

impl VersionedTable {
    fn new(name: &str, location: String, dataset: Arc<Dataset>) -> Self {
        Self {
            name: name.to_string(),
            location,
            dataset,
            versions: Mutex::default(),
            open_error: Mutex::default(),
        }
    }

    fn take_open_error(&self) -> Option<Error> {
        self.open_error.lock().unwrap().take()
    }

    /// N of `"<leaf>@v<N>"`, if `name` has that form.
    fn pinned_version(&self, name: &str) -> Option<u64> {
        name.strip_prefix(self.name.as_str())?
            .strip_prefix("@v")?
            .parse()
            .ok()
    }
}

impl std::fmt::Debug for VersionedTable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VersionedTable")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl SchemaProvider for VersionedTable {
    fn table_names(&self) -> Vec<String> {
        vec![self.name.clone()]
    }

    async fn table(
        &self,
        name: &str,
    ) -> std::result::Result<Option<Arc<dyn TableProvider>>, DataFusionError> {
        if name == self.name {
            return Ok(Some(table_provider(self.dataset.clone())));
        }
        let Some(version) = self.pinned_version(name) else {
            return Ok(None);
        };
        if let Some(provider) = self.versions.lock().unwrap().get(&version) {
            return Ok(Some(provider.clone()));
        }
        let dataset = self.dataset.checkout_version(version).await.map_err(|e| {
            let message = format!("cannot open version {version} of {}: {e}", self.location);
            let missing = matches!(
                e,
                lance::Error::DatasetNotFound { .. }
                    | lance::Error::NotFound { .. }
                    | lance::Error::VersionNotFound { .. }
            );
            let error = if missing {
                NamespaceError::TableVersionNotFound {
                    message: message.clone(),
                }
            } else {
                NamespaceError::Internal {
                    message: message.clone(),
                }
            };
            self.open_error
                .lock()
                .unwrap()
                .get_or_insert_with(|| error.into());
            DataFusionError::Plan(message)
        })?;
        let provider = table_provider(Arc::new(dataset));
        self.versions
            .lock()
            .unwrap()
            .insert(version, provider.clone());
        Ok(Some(provider))
    }

    fn table_exist(&self, name: &str) -> bool {
        name == self.name || self.pinned_version(name).is_some()
    }
}

/// Blob columns read as descriptors, so queries never fetch payload bytes.
fn table_provider(dataset: Arc<Dataset>) -> Arc<dyn TableProvider> {
    Arc::new(
        LanceTableProvider::new(dataset, false, false)
            .with_blob_handling(BlobHandling::BlobsDescriptions),
    )
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

fn sql_error(e: DataFusionError) -> Error {
    let message = format!("query failed: {e}");
    match e.find_root() {
        DataFusionError::ResourcesExhausted(message) => {
            invalid_input(format!("query exceeded the memory limit: {message}"))
        }
        DataFusionError::ObjectStore(err)
            if matches!(
                **err,
                lance::deps::datafusion::object_store::Error::NotFound { .. }
            ) =>
        {
            NamespaceError::TableVersionNotFound { message }.into()
        }
        DataFusionError::External(err)
            if err.downcast_ref::<lance::Error>().is_some_and(is_missing) =>
        {
            NamespaceError::TableVersionNotFound { message }.into()
        }
        DataFusionError::ObjectStore(_)
        | DataFusionError::IoError(_)
        | DataFusionError::External(_)
        | DataFusionError::Internal(_) => internal(message),
        _ => invalid_input(e.to_string()),
    }
}
