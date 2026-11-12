use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::Arc;

use async_trait::async_trait;
use futures::{StreamExt, TryStreamExt};
use lance::dataset::builder::DatasetBuilder;
use lance::dataset::statistics::DatasetStatisticsExt;
use lance::index::DatasetIndexExt;
use lance::io::ObjectStoreRegistry;
use lance::session::Session;
use lance::Dataset;
use lance_index::is_system_index;
use lance_namespace::models::{
    BranchContents, CreateNamespaceRequest, CreateNamespaceResponse, DeregisterTableRequest,
    DeregisterTableResponse, DescribeNamespaceRequest, DescribeNamespaceResponse,
    DescribeTableIndexStatsRequest, DescribeTableIndexStatsResponse, DescribeTableRequest,
    DescribeTableResponse, DescribeTableVersionRequest, DescribeTableVersionResponse,
    DropNamespaceRequest, DropNamespaceResponse, FragmentStats, FragmentSummary,
    GetTableStatsRequest, GetTableStatsResponse, IndexContent, ListNamespacesRequest,
    ListNamespacesResponse, ListTableBranchesRequest, ListTableBranchesResponse,
    ListTableIndicesRequest, ListTableIndicesResponse, ListTableTagsRequest, ListTableTagsResponse,
    ListTableVersionsRequest, ListTableVersionsResponse, ListTablesRequest, ListTablesResponse,
    NamespaceExistsRequest, RegisterTableRequest, RegisterTableResponse, TableBasicStats,
    TableExistsRequest, TableVersion, TagContents,
};
use lance_namespace::schema::arrow_schema_to_json;
use lance_namespace::{Error, LanceNamespace, NamespaceError, Result};
use lance_table::io::deletion::relative_deletion_file_path;
use sqlx::types::Json;
use sqlx::PgPool;

use crate::cache::{
    DeletedRows, FieldBytes, FragmentRows, LocationCache, OpenedDataset, StorageListing,
};

/// Path delimiter of REST ids. Names may not contain it, so joined ids stay unambiguous.
const DELIMITER: &str = "$";

/// Property keys stored in dedicated `tables` columns instead of the properties JSON.
const OWNER: &str = "owner";
const DESCRIPTION: &str = "description";
/// Server-managed property keys; clients may not set any key under the prefix.
const CATALOG_PREFIX: &str = "catalog.";
const REGISTERED_AT: &str = "catalog.registered_at";
const UPDATED_AT: &str = "catalog.updated_at";

/// `tables` columns read into [`TableRow`], with times rendered as RFC 3339 UTC.
const TABLE_COLUMNS: &str = r#"location, properties, owner, description, current_version, current_e_tag,
    to_char(registered_at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS registered_at,
    to_char(updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS updated_at"#;

/// Default total budget of the Lance session's caches.
pub const DEFAULT_CACHE_BUDGET_BYTES: usize = 256 << 20;

/// Fragments below the compaction target row count, as upstream `get_table_stats` counts them.
const SMALL_FRAGMENT_ROWS: u64 = 1024 * 1024;
/// Object-store reads (manifests, fragment footers) issued concurrently per request.
pub(crate) const READ_CONCURRENCY: usize = 16;

type Properties = HashMap<String, String>;

/// Bytes on disk of one top-level column, nested fields included.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnBytes {
    pub name: String,
    pub bytes_on_disk: u64,
}

/// Per-column storage of one table version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnStorage {
    pub version: u64,
    pub columns: Vec<ColumnBytes>,
}

/// Object-storage categories of a dataset prefix, in response order.
pub const STORAGE_CATEGORIES: [&str; 7] = [
    "data",
    "blob",
    "index",
    "deletion",
    "manifest",
    "transaction",
    "other",
];

/// Bytes and object count of one storage category.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageCategory {
    pub name: &'static str,
    pub bytes: u64,
    pub objects: u64,
}

/// Bytes and objects of a set of stored objects, by category.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageBreakdown {
    pub total_bytes: u64,
    pub total_objects: u64,
    /// One entry per [`STORAGE_CATEGORIES`] name, in that order.
    pub categories: Vec<StorageCategory>,
}

impl StorageBreakdown {
    fn new(counts: &[(u64, u64)]) -> Self {
        let categories: Vec<StorageCategory> = STORAGE_CATEGORIES
            .iter()
            .zip(counts)
            .map(|(&name, &(bytes, objects))| StorageCategory {
                name,
                bytes,
                objects,
            })
            .collect();
        Self {
            total_bytes: categories.iter().map(|c| c.bytes).sum(),
            total_objects: categories.iter().map(|c| c.objects).sum(),
            categories,
        }
    }
}

/// A table version's data next to everything stored under its location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageFootprint {
    pub version: u64,
    /// Data files the version references (manifest `total_files_size`).
    pub data_bytes: u64,
    /// Files of the version's indices.
    pub index_bytes: u64,
    /// Objects under the location that the version references.
    pub current: StorageBreakdown,
    /// Every object under the location, all versions included.
    pub all: StorageBreakdown,
}

/// Min, mean, max, and percentiles of one per-fragment measure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Distribution {
    pub min: u64,
    pub p25: u64,
    pub p50: u64,
    pub mean: u64,
    pub p75: u64,
    pub p99: u64,
    pub max: u64,
}

impl Distribution {
    /// `None` for no values. A percentile is the sorted value at `floor((n - 1) * p)`, as in
    /// Lance's `GetTableStats`; the mean rounds down.
    fn of(mut values: Vec<u64>) -> Option<Self> {
        values.sort_unstable();
        let (&min, &max) = (values.first()?, values.last()?);
        let pct = |p: f64| values[((values.len() - 1) as f64 * p) as usize];
        Some(Self {
            min,
            p25: pct(0.25),
            p50: pct(0.50),
            mean: values.iter().sum::<u64>() / values.len() as u64,
            p75: pct(0.75),
            p99: pct(0.99),
            max,
        })
    }
}

/// Per-fragment measures of one version. A measure is `None` when the table has no fragments or
/// some fragment's manifest entry does not record it (data file sizes from old writers, deletion
/// files without a row count).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FragmentMeasures {
    pub version: u64,
    pub fragments: u64,
    pub data_files: Option<Distribution>,
    pub data_bytes: Option<Distribution>,
    pub deleted_rows: Option<Distribution>,
}

/// Outcome of [`PostgresNamespace::refresh_table`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refresh {
    /// The current version before the refresh; `None` when none was recorded yet.
    pub previous_version: Option<u64>,
    pub current_version: u64,
}

#[derive(sqlx::FromRow)]
struct TableRow {
    location: String,
    properties: Json<Properties>,
    owner: Option<String>,
    description: Option<String>,
    current_version: Option<i64>,
    current_e_tag: Option<String>,
    registered_at: String,
    updated_at: String,
}

/// A registered table opened at the version a read targets.
pub(crate) struct OpenTable {
    pub(crate) location: String,
    pub(crate) dataset: Arc<Dataset>,
    cache: LocationCache,
}

impl OpenTable {
    fn e_tag(&self) -> Option<&str> {
        self.dataset.manifest_location().e_tag.as_deref()
    }

    /// Reads every data file's metadata on a miss.
    async fn field_bytes(&self) -> Result<Arc<FieldBytes>> {
        let (dataset, location) = (self.dataset.clone(), &self.location);
        self.cache
            .get_or_load(self.dataset.version_id(), self.e_tag(), || async move {
                let stats = dataset
                    .calculate_data_stats()
                    .await
                    .map_err(lance_error(format!("calculate data stats at {location}")))?;
                Ok(FieldBytes(
                    stats
                        .fields
                        .iter()
                        .map(|f| (f.id, f.bytes_on_disk))
                        .collect(),
                ))
            })
            .await
    }

    async fn fragment_rows(&self) -> Result<Arc<FragmentRows>> {
        let (dataset, location) = (self.dataset.clone(), &self.location);
        self.cache
            .get_or_load(self.dataset.version_id(), self.e_tag(), || async move {
                let mut rows: Vec<u64> = futures::stream::iter(dataset.get_fragments())
                    .map(|f| async move { f.physical_rows().await.map(|rows| rows as u64) })
                    .buffered(READ_CONCURRENCY)
                    .try_collect()
                    .await
                    .map_err(lance_error(format!("count fragment rows at {location}")))?;
                rows.sort_unstable();
                Ok(FragmentRows(rows))
            })
            .await
    }

    async fn deleted_rows(&self) -> Result<usize> {
        let (dataset, location) = (self.dataset.clone(), &self.location);
        let deleted = self
            .cache
            .get_or_load(self.dataset.version_id(), self.e_tag(), || async move {
                let rows = dataset
                    .count_deleted_rows()
                    .await
                    .map_err(lance_error(format!("count deleted rows at {location}")))?;
                Ok(DeletedRows(rows))
            })
            .await?;
        Ok(deleted.0)
    }

    /// Every object under the location, bucketed once over all objects and once over the
    /// objects this version references.
    async fn storage_listing(&self) -> Result<Arc<StorageListing>> {
        let (dataset, location) = (self.dataset.clone(), &self.location);
        self.cache
            .get_or_load(self.dataset.version_id(), self.e_tag(), || async move {
                let store = dataset
                    .object_store(None)
                    .await
                    .map_err(lance_error(format!("open object store at {location}")))?;
                let referenced = VersionFiles::of(&dataset)
                    .await
                    .map_err(lance_error(format!("load indices at {location}")))?;
                let base = dataset.branch_location().path;
                let mut all = vec![(0u64, 0u64); STORAGE_CATEGORIES.len()];
                let mut current = all.clone();
                let mut objects = store.read_dir_all(&base, None);
                while let Some(meta) = objects
                    .try_next()
                    .await
                    .map_err(lance_error(format!("list objects at {location}")))?
                {
                    let path = meta.location.as_ref();
                    let relative = path
                        .strip_prefix(base.as_ref())
                        .map_or(path, |p| p.trim_start_matches('/'));
                    let category = storage_category(relative);
                    all[category].0 += meta.size;
                    all[category].1 += 1;
                    if referenced.contains(relative) {
                        current[category].0 += meta.size;
                        current[category].1 += 1;
                    }
                }
                Ok(StorageListing { current, all })
            })
            .await
    }
}

impl TableRow {
    /// The properties as the client registered them.
    fn client_properties(&self) -> Properties {
        let mut properties = self.properties.0.clone();
        for (key, value) in [(OWNER, &self.owner), (DESCRIPTION, &self.description)] {
            if let Some(value) = value {
                properties.insert(key.to_string(), value.clone());
            }
        }
        properties
    }

    fn properties_with_times(&self) -> Properties {
        let mut properties = self.client_properties();
        properties.insert(REGISTERED_AT.to_string(), self.registered_at.clone());
        properties.insert(UPDATED_AT.to_string(), self.updated_at.clone());
        properties
    }
}

/// A [`LanceNamespace`] whose namespaces and table registrations live in Postgres.
///
/// Postgres stores identity, location, properties, registration times, and the current version:
/// the version every read that names no version answers for. Lance manifests stay
/// authoritative for each version's contents. Values derived from one version are cached in the
/// Lance session under its budget. `storage_options` are used to open datasets server-side and
/// are never returned to callers.
pub struct PostgresNamespace {
    pool: PgPool,
    storage_options: HashMap<String, String>,
    session: Arc<Session>,
    /// Names allowed for new top-level namespaces; empty allows any.
    top_level_allowlist: Vec<String>,
}

impl std::fmt::Debug for PostgresNamespace {
    // Hand-written so credentials in `storage_options` never reach logs.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostgresNamespace").finish_non_exhaustive()
    }
}

impl PostgresNamespace {
    pub fn new(pool: PgPool, storage_options: HashMap<String, String>) -> Self {
        Self {
            pool,
            storage_options,
            session: session(DEFAULT_CACHE_BUDGET_BYTES),
            top_level_allowlist: Vec::new(),
        }
    }

    /// Bounds the Lance session's caches by `bytes` in total, replacing the default budget.
    pub fn with_cache_budget(mut self, bytes: usize) -> Self {
        self.session = session(bytes);
        self
    }

    /// Restricts creating top-level namespaces to `names`; an empty list allows any. Existing
    /// top-level namespaces outside the list stay readable and droppable. Errors with
    /// `InvalidInput` when a name is not a valid namespace name.
    pub fn with_top_level_allowlist(mut self, names: Vec<String>) -> Result<Self> {
        validate_names(&names)?;
        self.top_level_allowlist = names;
        Ok(self)
    }

    /// True when the database answers a trivial query.
    pub async fn ping(&self) -> bool {
        sqlx::query("SELECT 1").execute(&self.pool).await.is_ok()
    }

    async fn open_dataset(&self, location: &str, version: Option<u64>) -> lance::Result<Dataset> {
        let mut builder = DatasetBuilder::from_uri(location)
            .with_storage_options(self.storage_options.clone())
            .with_session(self.session.clone());
        if let Some(version) = version {
            builder = builder.with_version(version);
        }
        builder.load().await
    }

    /// The newest version, reopened by number so its manifest e-tag comes from the same lookup
    /// later versioned opens make; resolving "latest" can report the e-tag differently.
    async fn open_newest(&self, location: &str) -> lance::Result<Dataset> {
        let latest = self.open_dataset(location, None).await?;
        self.open_dataset(location, Some(latest.version_id())).await
    }

    /// Per top-level column bytes on disk of the current version; nested fields roll up into
    /// their top-level column. Reads every data file's metadata once per version and instance.
    pub async fn column_storage(&self, id: &[String]) -> Result<ColumnStorage> {
        self.read_table(id, None, |table| async move {
            let bytes = table.field_bytes().await?;
            let fields = &table.dataset.schema().fields;
            let mut top_level = HashMap::new();
            for (index, field) in fields.iter().enumerate() {
                let mut stack = vec![field];
                while let Some(f) = stack.pop() {
                    top_level.insert(f.id as u32, index);
                    stack.extend(f.children.iter());
                }
            }
            let mut totals = vec![0u64; fields.len()];
            for &(id, bytes_on_disk) in &bytes.0 {
                if let Some(&index) = top_level.get(&id) {
                    totals[index] += bytes_on_disk;
                }
            }
            Ok(ColumnStorage {
                version: table.dataset.version_id(),
                columns: fields
                    .iter()
                    .zip(totals)
                    .map(|(f, bytes_on_disk)| ColumnBytes {
                        name: f.name.clone(),
                        bytes_on_disk,
                    })
                    .collect(),
            })
        })
        .await
    }

    /// Lists every object under the registered location (metadata only, never writes) once per
    /// current version and instance, bucketed by path relative to that location, both overall
    /// and for the objects the current version references. The prefix match is per path
    /// segment, so sibling directories such as `<table>.lance_tags/` are excluded.
    pub async fn storage_footprint(&self, id: &[String]) -> Result<StorageFootprint> {
        self.read_table(id, None, |table| async move {
            let location = &table.location;
            let index_bytes = async {
                let descriptions = table
                    .dataset
                    .describe_indices(None)
                    .await
                    .map_err(lance_error(format!("describe indices at {location}")))?;
                Ok::<u64, Error>(
                    descriptions
                        .iter()
                        .filter(|d| d.metadata().first().is_some_and(|m| !is_system_index(m)))
                        .filter_map(|d| d.total_size_bytes())
                        .sum(),
                )
            };
            let (index_bytes, listing) = futures::try_join!(index_bytes, table.storage_listing())?;
            Ok(StorageFootprint {
                version: table.dataset.version_id(),
                data_bytes: table.dataset.manifest().summary().total_files_size,
                index_bytes,
                current: StorageBreakdown::new(&listing.current),
                all: StorageBreakdown::new(&listing.all),
            })
        })
        .await
    }

    /// Per-fragment data files, data bytes, and deleted rows of the current version, read from
    /// its manifest alone.
    pub async fn fragment_measures(&self, id: &[String]) -> Result<FragmentMeasures> {
        self.read_table(id, None, |table| async move {
            let fragments = table.dataset.manifest().fragments.clone();
            let data_files = fragments.iter().map(|f| Some(f.files.len() as u64));
            let data_bytes = fragments.iter().map(|f| {
                f.files
                    .iter()
                    .map(|file| file.file_size_bytes.get().map(u64::from))
                    .sum::<Option<u64>>()
            });
            let deleted_rows = fragments.iter().map(|f| match &f.deletion_file {
                None => Some(0),
                Some(d) => d.num_deleted_rows.map(|n| n as u64),
            });
            let measure = |values: Option<Vec<u64>>| values.and_then(Distribution::of);
            Ok(FragmentMeasures {
                version: table.dataset.version_id(),
                fragments: fragments.len() as u64,
                data_files: measure(data_files.collect()),
                data_bytes: measure(data_bytes.collect()),
                deleted_rows: measure(deleted_rows.collect()),
            })
        })
        .await
    }

    /// Advances the current version to the newest version at the registered location and
    /// returns the previous and new current versions.
    pub async fn refresh_table(&self, id: &[String]) -> Result<Refresh> {
        let row = self.table_row(id).await?;
        let latest = self.open_latest(&row.location).await?;
        let (namespace, name) = split_table_id(id)?;
        let updated = sqlx::query(
            "UPDATE tables SET current_version = $4, current_e_tag = $5
             WHERE namespace_path = $1 AND name = $2 AND location = $3",
        )
        .bind(namespace)
        .bind(name)
        .bind(&row.location)
        .bind(latest.dataset.version_id() as i64)
        .bind(latest.e_tag())
        .execute(&self.pool)
        .await
        .map_err(db_error)?;
        if updated.rows_affected() == 0 {
            return Err(concurrent_registration(id));
        }
        Ok(Refresh {
            previous_version: row.current_version.map(|v| v as u64),
            current_version: latest.dataset.version_id(),
        })
    }

    async fn table_row(&self, id: &[String]) -> Result<TableRow> {
        let (namespace, name) = split_table_id(id)?;
        sqlx::query_as(&format!(
            "SELECT {TABLE_COLUMNS} FROM tables WHERE namespace_path = $1 AND name = $2"
        ))
        .bind(namespace)
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(db_error)?
        .ok_or_else(|| table_not_found(id))
    }

    /// Opens the table at `version`, or at its current version when unset. Unlike
    /// [`Self::read_table`], a read that fails afterwards does not advance the current version.
    pub(crate) async fn open_table(
        &self,
        id: &[String],
        version: Option<u64>,
    ) -> Result<OpenTable> {
        let row = self.table_row(id).await?;
        Ok(self.open_row(id, &row, version).await?.0)
    }

    /// The table at `version`, or at its current version when unset; also whether this is an
    /// unversioned read of the current version, the only kind that may advance it.
    async fn open_row(
        &self,
        id: &[String],
        row: &TableRow,
        version: Option<u64>,
    ) -> Result<(OpenTable, bool)> {
        let Some(version) = version else {
            return Ok((self.open_current(id, row).await?, true));
        };
        if row.current_version != Some(version as i64) {
            return Ok((self.open_version(&row.location, version).await?, false));
        }
        // A pinned read of the current version still uses the cache, but never advances.
        let table = self
            .open_cached(&row.location, version, row.current_e_tag.as_deref())
            .await
            .map_err(lance_error(format!(
                "cannot open Lance dataset at {}",
                row.location
            )))?;
        Ok((table, false))
    }

    /// Runs `read` on the table at `version`, or at its current version when unset.
    ///
    /// A current version whose files are gone (cleanup removes old manifests and the data files
    /// only they reference) advances to the newest version, and `read` runs once more there.
    pub(crate) async fn read_table<T, F, Fut>(
        &self,
        id: &[String],
        version: Option<u64>,
        read: F,
    ) -> Result<T>
    where
        F: Fn(OpenTable) -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        let row = self.table_row(id).await?;
        self.read_row(id, &row, version, read).await
    }

    async fn read_row<T, F, Fut>(
        &self,
        id: &[String],
        row: &TableRow,
        version: Option<u64>,
        read: F,
    ) -> Result<T>
    where
        F: Fn(OpenTable) -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        let (table, current) = self.open_row(id, row, version).await?;
        let opened_version = table.dataset.version_id();
        match read(table).await {
            Err(e)
                if current
                    && is_version_missing(&e)
                    && self.version_gone(row, opened_version).await =>
            {
                let row = self.table_row(id).await?;
                read(self.advance(id, &row).await?).await
            }
            result => result,
        }
    }

    /// The current version, from this instance's cache when present. A missing or unrecorded
    /// current version advances to the newest one.
    async fn open_current(&self, id: &[String], row: &TableRow) -> Result<OpenTable> {
        let Some(version) = row.current_version.map(|v| v as u64) else {
            return self.advance(id, row).await;
        };
        match self
            .open_cached(&row.location, version, row.current_e_tag.as_deref())
            .await
        {
            Ok(table) => Ok(table),
            Err(e) if is_missing(&e) => self.advance(id, row).await,
            Err(e) => Err(lance_error(format!(
                "cannot open Lance dataset at {}",
                row.location
            ))(e)),
        }
    }

    /// Records the newest version as current, unless another request already moved the current
    /// version past what `row` holds; then serves what that request recorded.
    async fn advance(&self, id: &[String], row: &TableRow) -> Result<OpenTable> {
        let latest = self.open_latest(&row.location).await?;
        let (namespace, name) = split_table_id(id)?;
        let updated = sqlx::query(
            "UPDATE tables SET current_version = $4, current_e_tag = $5
             WHERE namespace_path = $1 AND name = $2 AND location = $3
               AND current_version IS NOT DISTINCT FROM $6
               AND current_e_tag IS NOT DISTINCT FROM $7",
        )
        .bind(namespace)
        .bind(name)
        .bind(&row.location)
        .bind(latest.dataset.version_id() as i64)
        .bind(latest.e_tag())
        .bind(row.current_version)
        .bind(&row.current_e_tag)
        .execute(&self.pool)
        .await
        .map_err(db_error)?;
        if updated.rows_affected() == 1 {
            return Ok(latest);
        }
        let current = self.table_row(id).await?;
        match current.current_version.map(|v| v as u64) {
            Some(version) => self
                .open_cached(&current.location, version, current.current_e_tag.as_deref())
                .await
                .map_err(lance_error(format!(
                    "cannot open Lance dataset at {}",
                    current.location
                ))),
            None => Err(concurrent_registration(id)),
        }
    }

    /// Whether `version`'s manifest no longer exists; probes object storage, so it is only
    /// asked after a read already failed.
    async fn version_gone(&self, row: &TableRow, version: u64) -> bool {
        matches!(
            self.open_dataset(&row.location, Some(version)).await,
            Err(e) if is_missing(&e)
        )
    }

    /// The newest version, cached under its own e-tag. A dataset with no manifest at all is
    /// TableNotFound, so callers can tell a vanished dataset from a failing one.
    async fn open_latest(&self, location: &str) -> Result<OpenTable> {
        let dataset = self.open_newest(location).await.map_err(|e| {
            let message = format!("cannot open Lance dataset at {location}: {e}");
            if is_missing(&e) {
                NamespaceError::TableNotFound { message }.into()
            } else {
                internal(message)
            }
        })?;
        let table = self.table_handle(location, dataset);
        table
            .cache
            .insert(
                table.dataset.version_id(),
                table.e_tag(),
                OpenedDataset(table.dataset.clone()),
            )
            .await;
        Ok(table)
    }

    async fn open_version(&self, location: &str, version: u64) -> Result<OpenTable> {
        let dataset = self
            .open_dataset(location, Some(version))
            .await
            .map_err(lance_error(format!(
                "cannot open Lance dataset at {location}"
            )))?;
        Ok(self.table_handle(location, dataset))
    }

    /// `version` from this instance's cache, opened and cached on a miss.
    async fn open_cached(
        &self,
        location: &str,
        version: u64,
        e_tag: Option<&str>,
    ) -> lance::Result<OpenTable> {
        let cache = LocationCache::new(&self.session, location);
        let dataset = match cache.get::<OpenedDataset>(version, e_tag).await {
            Some(opened) => opened.0.clone(),
            None => {
                let dataset = Arc::new(self.open_dataset(location, Some(version)).await?);
                // A dataset recreated at the same location reuses version numbers; a different
                // e-tag means the recorded version no longer exists.
                let opened = dataset.manifest_location().e_tag.as_deref();
                if let (Some(expected), Some(opened)) = (e_tag, opened) {
                    if expected != opened {
                        return Err(lance::Error::not_found(format!(
                            "{location} version {version} with e-tag {expected}"
                        )));
                    }
                }
                cache
                    .insert(version, e_tag, OpenedDataset(dataset.clone()))
                    .await;
                dataset
            }
        };
        Ok(OpenTable {
            location: location.to_string(),
            dataset,
            cache,
        })
    }

    fn table_handle(&self, location: &str, dataset: Dataset) -> OpenTable {
        OpenTable {
            location: location.to_string(),
            dataset: Arc::new(dataset),
            cache: LocationCache::new(&self.session, location),
        }
    }

    /// Fails with NamespaceNotFound unless `path` is the root or a stored namespace.
    async fn require_namespace(&self, path: &[String]) -> Result<()> {
        if path.is_empty() || self.namespace_row_exists(path).await? {
            Ok(())
        } else {
            Err(namespace_not_found(path))
        }
    }

    async fn namespace_row_exists(&self, path: &[String]) -> Result<bool> {
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM namespaces WHERE path = $1)")
            .bind(path)
            .fetch_one(&self.pool)
            .await
            .map_err(db_error)
    }
}

#[async_trait]
impl LanceNamespace for PostgresNamespace {
    fn namespace_id(&self) -> String {
        let options = self.pool.connect_options();
        format!(
            "postgres(host={}, database={})",
            options.get_host(),
            options.get_database().unwrap_or_default()
        )
    }

    async fn list_namespaces(
        &self,
        request: ListNamespacesRequest,
    ) -> Result<ListNamespacesResponse> {
        let parent = request.id.unwrap_or_default();
        self.require_namespace(&parent).await?;
        let page = Page::new(request.page_token, request.limit);
        let names: Vec<String> = sqlx::query_scalar(
            r#"SELECT path[cardinality(path)] FROM namespaces
               WHERE parent_path IS NOT DISTINCT FROM $1
                 AND ($2::text IS NULL OR path[cardinality(path)] COLLATE "C" > $2)
               ORDER BY path[cardinality(path)] COLLATE "C"
               LIMIT $3"#,
        )
        .bind((!parent.is_empty()).then_some(&parent))
        .bind(&page.start_after)
        .bind(page.fetch_limit())
        .fetch_all(&self.pool)
        .await
        .map_err(db_error)?;
        let (namespaces, page_token) = page.finish(names);
        Ok(ListNamespacesResponse {
            namespaces,
            page_token,
            ..Default::default()
        })
    }

    async fn describe_namespace(
        &self,
        request: DescribeNamespaceRequest,
    ) -> Result<DescribeNamespaceResponse> {
        let path = request.id.unwrap_or_default();
        if path.is_empty() {
            return Ok(DescribeNamespaceResponse::default());
        }
        let properties: Option<Json<Properties>> =
            sqlx::query_scalar("SELECT properties FROM namespaces WHERE path = $1")
                .bind(&path)
                .fetch_optional(&self.pool)
                .await
                .map_err(db_error)?;
        let Json(properties) = properties.ok_or_else(|| namespace_not_found(&path))?;
        Ok(DescribeNamespaceResponse {
            properties: Some(properties),
            ..Default::default()
        })
    }

    async fn create_namespace(
        &self,
        request: CreateNamespaceRequest,
    ) -> Result<CreateNamespaceResponse> {
        let path = request.id.unwrap_or_default();
        let mode = normalize_mode(request.mode.as_deref(), "create");
        if !matches!(mode.as_str(), "create" | "existok" | "overwrite") {
            return Err(invalid_input(format!(
                "unknown create mode {:?}",
                request.mode
            )));
        }
        if path.is_empty() {
            return Err(NamespaceError::NamespaceAlreadyExists {
                message: "the root namespace always exists".into(),
            }
            .into());
        }
        validate_names(&path)?;
        if let [name] = path.as_slice() {
            if !self.top_level_allowlist.is_empty() && !self.top_level_allowlist.contains(name) {
                return Err(invalid_input(format!(
                    "top-level namespace {name:?} is not allowed; allowed: {}",
                    self.top_level_allowlist.join(", ")
                )));
            }
        }
        let properties = Json(request.properties.unwrap_or_default());

        let mut tx = self.pool.begin().await.map_err(db_error)?;
        if mode == "overwrite" {
            // Spec: overwrite drops the existing namespace and creates an empty one; the
            // RESTRICT foreign keys refuse when it still has children.
            sqlx::query("DELETE FROM namespaces WHERE path = $1")
                .bind(&path)
                .execute(&mut *tx)
                .await
                .map_err(|e| on_foreign_key(e, || namespace_not_empty(&path)))?;
        }
        let inserted: Option<Json<Properties>> = sqlx::query_scalar(
            "INSERT INTO namespaces (path, properties) VALUES ($1, $2)
             ON CONFLICT (path) DO NOTHING RETURNING properties",
        )
        .bind(&path)
        .bind(&properties)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| on_foreign_key(e, || parent_not_found(&path)))?;
        let properties = match inserted {
            Some(Json(p)) => p,
            None if mode == "existok" => {
                let Json(existing): Json<Properties> =
                    sqlx::query_scalar("SELECT properties FROM namespaces WHERE path = $1")
                        .bind(&path)
                        .fetch_one(&mut *tx)
                        .await
                        .map_err(db_error)?;
                existing
            }
            None => {
                return Err(NamespaceError::NamespaceAlreadyExists {
                    message: join_id(&path),
                }
                .into())
            }
        };
        tx.commit().await.map_err(db_error)?;
        Ok(CreateNamespaceResponse {
            properties: Some(properties),
            ..Default::default()
        })
    }

    async fn drop_namespace(&self, request: DropNamespaceRequest) -> Result<DropNamespaceResponse> {
        let path = request.id.unwrap_or_default();
        let mode = normalize_mode(request.mode.as_deref(), "fail");
        let skip_missing = match mode.as_str() {
            "fail" => false,
            "skip" => true,
            _ => {
                return Err(invalid_input(format!(
                    "unknown drop mode {:?}",
                    request.mode
                )))
            }
        };
        match normalize_mode(request.behavior.as_deref(), "restrict").as_str() {
            "restrict" => {}
            "cascade" => {
                return Err(NamespaceError::Unsupported {
                    message: "cascading namespace drop is not supported".into(),
                }
                .into())
            }
            _ => {
                return Err(invalid_input(format!(
                    "unknown drop behavior {:?}",
                    request.behavior
                )))
            }
        }
        if path.is_empty() {
            return Err(invalid_input("the root namespace cannot be dropped".into()));
        }
        let dropped: Option<Json<Properties>> =
            sqlx::query_scalar("DELETE FROM namespaces WHERE path = $1 RETURNING properties")
                .bind(&path)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| on_foreign_key(e, || namespace_not_empty(&path)))?;
        match dropped {
            Some(Json(properties)) => Ok(DropNamespaceResponse {
                properties: Some(properties),
                ..Default::default()
            }),
            None if skip_missing => Ok(DropNamespaceResponse::default()),
            None => Err(namespace_not_found(&path)),
        }
    }

    async fn namespace_exists(&self, request: NamespaceExistsRequest) -> Result<()> {
        self.require_namespace(&request.id.unwrap_or_default())
            .await
    }

    async fn list_tables(&self, request: ListTablesRequest) -> Result<ListTablesResponse> {
        let namespace = request.id.unwrap_or_default();
        self.require_namespace(&namespace).await?;
        let page = Page::new(request.page_token, request.limit);
        let names: Vec<String> = sqlx::query_scalar(
            r#"SELECT name FROM tables
               WHERE namespace_path = $1 AND ($2::text IS NULL OR name COLLATE "C" > $2)
               ORDER BY name COLLATE "C"
               LIMIT $3"#,
        )
        .bind(&namespace)
        .bind(&page.start_after)
        .bind(page.fetch_limit())
        .fetch_all(&self.pool)
        .await
        .map_err(db_error)?;
        let (tables, page_token) = page.finish(names);
        Ok(ListTablesResponse {
            tables,
            page_token,
            ..Default::default()
        })
    }

    /// Lists every registered table as its full `$`-joined id (for example `prod$users`), which
    /// is also the REST path id of the table.
    async fn list_all_tables(&self, request: ListTablesRequest) -> Result<ListTablesResponse> {
        let page = Page::new(request.page_token, request.limit);
        let ids: Vec<String> = sqlx::query_scalar(
            r#"SELECT full_id FROM (
                   SELECT array_to_string(namespace_path || name, '$') COLLATE "C" AS full_id
                   FROM tables
               ) t
               WHERE $1::text IS NULL OR full_id > $1
               ORDER BY full_id
               LIMIT $2"#,
        )
        .bind(&page.start_after)
        .bind(page.fetch_limit())
        .fetch_all(&self.pool)
        .await
        .map_err(db_error)?;
        let (tables, page_token) = page.finish(ids);
        Ok(ListTablesResponse {
            tables,
            page_token,
            ..Default::default()
        })
    }

    async fn describe_table(&self, request: DescribeTableRequest) -> Result<DescribeTableResponse> {
        if request.tag.is_some() || request.branch.is_some() {
            return Err(NamespaceError::Unsupported {
                message: "describing a tag or branch is not supported".into(),
            }
            .into());
        }
        let version = parse_version(request.version)?;
        let id = request.id.unwrap_or_default();
        let (namespace, name) = split_table_id(&id)?;
        let row = self.table_row(&id).await?;

        let mut response = DescribeTableResponse {
            table: Some(name.to_string()),
            namespace: Some(namespace.to_vec()),
            location: Some(row.location.clone()),
            table_uri: Some(row.location.clone()),
            properties: Some(row.properties_with_times()),
            ..Default::default()
        };
        let detailed = request.load_detailed_metadata.unwrap_or(false);
        if detailed || request.check_declared.unwrap_or(false) {
            // Registration requires an existing dataset, so no table is ever declared-only.
            response.is_only_declared = Some(false);
        }
        if !detailed && version.is_none() {
            return Ok(response);
        }

        let (row, response) = (&row, &response);
        self.read_row(&id, row, version, |table| async move {
            let dataset = &table.dataset;
            let arrow_schema: arrow_schema::Schema = dataset.schema().into();
            let num_deleted_rows = table.deleted_rows().await?;
            let lance_version = dataset.version();
            let mut metadata: Properties = lance_version.metadata.into_iter().collect();
            metadata.extend(
                dataset
                    .config()
                    .iter()
                    .map(|(key, value)| (format!("config.{key}"), value.clone())),
            );
            metadata.extend(
                dataset
                    .metadata()
                    .iter()
                    .map(|(key, value)| (format!("table_metadata.{key}"), value.clone())),
            );
            metadata.insert(
                "lance.data_storage_version".into(),
                dataset.manifest().data_storage_format.version.to_string(),
            );
            let mut response = response.clone();
            response.version = Some(lance_version.version as i64);
            response.schema = Some(Box::new(arrow_schema_to_json(&arrow_schema)?));
            response.stats = Some(Box::new(TableBasicStats::new(
                saturating_i32(num_deleted_rows),
                saturating_i32(dataset.count_fragments()),
            )));
            response.metadata = Some(metadata);
            Ok(response)
        })
        .await
    }

    async fn register_table(&self, request: RegisterTableRequest) -> Result<RegisterTableResponse> {
        let overwrite = match normalize_mode(request.mode.as_deref(), "create").as_str() {
            "create" => false,
            "overwrite" => true,
            _ => {
                return Err(invalid_input(format!(
                    "unknown register mode {:?}",
                    request.mode
                )))
            }
        };
        let id = request.id.unwrap_or_default();
        let (namespace, name) = split_table_id(&id)?;
        if namespace.is_empty() {
            return Err(invalid_input(
                "tables must be registered under a namespace, not the root".into(),
            ));
        }
        validate_names(&id)?;
        if request.location.is_empty() {
            return Err(invalid_input("location is required".into()));
        }

        // Cheap checks first so a missing namespace or duplicate never waits on object storage;
        // the insert below still enforces both under concurrency.
        self.require_namespace(namespace).await?;
        if !overwrite && self.table_row_exists(namespace, name).await? {
            return Err(table_already_exists(&id));
        }
        let dataset = self.open_newest(&request.location).await.map_err(|e| {
            invalid_input(format!(
                "cannot open Lance dataset at {}: {e}",
                request.location
            ))
        })?;
        let current_version = dataset.version_id() as i64;
        let current_e_tag = dataset.manifest_location().e_tag.clone();

        let client_properties = request.properties.unwrap_or_default();
        if let Some(key) = client_properties
            .keys()
            .find(|key| key.starts_with(CATALOG_PREFIX))
        {
            return Err(invalid_input(format!(
                "property {key:?} is managed by the catalog; keys under {CATALOG_PREFIX:?} are reserved"
            )));
        }
        let mut properties = client_properties.clone();
        let owner = properties.remove(OWNER);
        let description = properties.remove(DESCRIPTION);
        // Overwrite advances updated_at and keeps registered_at.
        let sql = if overwrite {
            "INSERT INTO tables (namespace_path, name, location, properties, owner, description,
                                 current_version, current_e_tag)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (namespace_path, name)
             DO UPDATE SET location = EXCLUDED.location, properties = EXCLUDED.properties,
                 owner = EXCLUDED.owner, description = EXCLUDED.description,
                 current_version = EXCLUDED.current_version, current_e_tag = EXCLUDED.current_e_tag,
                 updated_at = now()"
        } else {
            "INSERT INTO tables (namespace_path, name, location, properties, owner, description,
                                 current_version, current_e_tag)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"
        };
        sqlx::query(sql)
            .bind(namespace)
            .bind(name)
            .bind(&request.location)
            .bind(Json(&properties))
            .bind(owner)
            .bind(description)
            .bind(current_version)
            .bind(current_e_tag)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                if e.as_database_error()
                    .is_some_and(|d| d.is_unique_violation())
                {
                    table_already_exists(&id)
                } else {
                    on_foreign_key(e, || namespace_not_found(namespace))
                }
            })?;
        Ok(RegisterTableResponse {
            location: Some(request.location),
            properties: Some(client_properties),
            ..Default::default()
        })
    }

    async fn table_exists(&self, request: TableExistsRequest) -> Result<()> {
        if request.version.is_some() {
            return Err(NamespaceError::Unsupported {
                message: "checking a table version is not supported".into(),
            }
            .into());
        }
        let id = request.id.unwrap_or_default();
        let (namespace, name) = split_table_id(&id)?;
        if self.table_row_exists(namespace, name).await? {
            Ok(())
        } else {
            Err(table_not_found(&id))
        }
    }

    async fn deregister_table(
        &self,
        request: DeregisterTableRequest,
    ) -> Result<DeregisterTableResponse> {
        let id = request.id.unwrap_or_default();
        let (namespace, name) = split_table_id(&id)?;
        let row: Option<TableRow> = sqlx::query_as(&format!(
            "DELETE FROM tables WHERE namespace_path = $1 AND name = $2 RETURNING {TABLE_COLUMNS}"
        ))
        .bind(namespace)
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(db_error)?;
        let row = row.ok_or_else(|| table_not_found(&id))?;
        Ok(DeregisterTableResponse {
            id: Some(id),
            properties: Some(row.client_properties()),
            location: Some(row.location),
            ..Default::default()
        })
    }

    /// Mirrors upstream `DirectoryNamespace::get_table_stats` for the current version: total
    /// bytes from field-level data statistics, fragment lengths from per-fragment physical rows.
    /// Both are computed once per version and instance.
    async fn get_table_stats(
        &self,
        request: GetTableStatsRequest,
    ) -> Result<GetTableStatsResponse> {
        reject_branch(request.branch.as_deref())?;
        let id = request.id.unwrap_or_default();
        self.read_table(&id, None, |table| async move {
            let location = &table.location;
            let indices = async {
                table
                    .dataset
                    .load_indices()
                    .await
                    .map_err(lance_error(format!("load indices at {location}")))
            };
            let (field_bytes, rows, indices) =
                futures::try_join!(table.field_bytes(), table.fragment_rows(), indices)?;
            let total_bytes: u64 = field_bytes.0.iter().map(|f| f.1).sum();
            let lengths = &rows.0;
            let num_rows: u64 = lengths.iter().sum();
            let num_small_fragments = lengths.iter().filter(|&&n| n < SMALL_FRAGMENT_ROWS).count();
            let summary = match Distribution::of(lengths.clone()) {
                Some(d) => FragmentSummary::new(
                    d.min as i64,
                    d.max as i64,
                    d.mean as i64,
                    d.p25 as i64,
                    d.p50 as i64,
                    d.p75 as i64,
                    d.p99 as i64,
                ),
                None => FragmentSummary::new(0, 0, 0, 0, 0, 0, 0),
            };
            let num_indices = indices.iter().filter(|m| !is_system_index(m)).count();
            Ok(GetTableStatsResponse::new(
                total_bytes as i64,
                num_rows as i64,
                num_indices as i64,
                FragmentStats::new(lengths.len() as i64, num_small_fragments as i64, summary),
            ))
        })
        .await
    }

    /// Start-after pagination by version number in the requested order; the token is the last
    /// version of the page. Lists every retained version, newer than current ones included; only
    /// the page's manifests are read.
    async fn list_table_versions(
        &self,
        request: ListTableVersionsRequest,
    ) -> Result<ListTableVersionsResponse> {
        reject_branch(request.branch.as_deref())?;
        let id = request.id.unwrap_or_default();
        let descending = request.descending.unwrap_or(false);
        let after = match request.page_token.filter(|t| !t.is_empty()) {
            Some(token) => Some(
                token
                    .parse::<u64>()
                    .map_err(|_| invalid_input(format!("invalid page token {token:?}")))?,
            ),
            None => None,
        };
        let limit = request.limit.and_then(|l| usize::try_from(l).ok());
        self.read_table(&id, None, |table| async move {
            let (location, dataset) = (&table.location, &table.dataset);
            let mut versions: Vec<u64> = dataset
                .version_refs()
                .await
                .map_err(lance_error(format!("list versions at {location}")))?
                .into_iter()
                .map(|v| v.version)
                .collect();
            if descending {
                versions.reverse();
            }
            if let Some(after) = after {
                versions.retain(|&v| if descending { v < after } else { v > after });
            }
            let mut page_token = None;
            if let Some(limit) = limit {
                if versions.len() > limit {
                    versions.truncate(limit);
                    page_token = versions.last().map(u64::to_string);
                }
            }
            let versions = futures::stream::iter(versions)
                .map(|version| async move {
                    if version == dataset.version_id() {
                        return Ok(table_version(dataset));
                    }
                    dataset
                        .checkout_version(version)
                        .await
                        .map(|checked_out| table_version(&checked_out))
                })
                .buffered(READ_CONCURRENCY)
                .try_collect()
                .await
                .map_err(|e| internal(format!("read versions at {location}: {e}")))?;
            Ok(ListTableVersionsResponse {
                versions,
                page_token,
                ..Default::default()
            })
        })
        .await
    }

    /// Describes `version`, or the current version when it is unset.
    async fn describe_table_version(
        &self,
        request: DescribeTableVersionRequest,
    ) -> Result<DescribeTableVersionResponse> {
        reject_branch(request.branch.as_deref())?;
        let version = parse_version(request.version)?;
        let OpenTable { dataset, .. } = self
            .open_table(&request.id.unwrap_or_default(), version)
            .await?;
        Ok(DescribeTableVersionResponse {
            version: Box::new(table_version(&dataset)),
            ..Default::default()
        })
    }

    /// Mirrors upstream `DirectoryNamespace::list_table_indices`, hiding system indices.
    async fn list_table_indices(
        &self,
        request: ListTableIndicesRequest,
    ) -> Result<ListTableIndicesResponse> {
        reject_branch(request.branch.as_deref())?;
        let version = parse_version(request.version)?;
        let OpenTable {
            location, dataset, ..
        } = self
            .open_table(&request.id.unwrap_or_default(), version)
            .await?;
        let total_rows = dataset
            .count_rows(None)
            .await
            .map_err(lance_error(format!("count rows at {location}")))?
            as u64;
        let descriptions = dataset
            .describe_indices(None)
            .await
            .map_err(lance_error(format!("describe indices at {location}")))?;
        let schema = dataset.schema();
        let mut indexes = Vec::new();
        for description in descriptions {
            let Some(first) = description.metadata().first() else {
                continue;
            };
            if is_system_index(first) {
                continue;
            }
            // A field id that no longer resolves is labelled by its id rather than failing
            // the whole listing.
            let columns = description
                .field_ids()
                .iter()
                .map(|&field_id| {
                    i32::try_from(field_id)
                        .ok()
                        .and_then(|field_id| schema.field_path_minimal(field_id).ok())
                        .unwrap_or_else(|| field_id.to_string())
                })
                .collect();
            let segments = description.segments();
            indexes.push(IndexContent {
                index_name: description.name().to_string(),
                index_uuid: first.uuid.to_string(),
                columns,
                status: "SUCCEEDED".to_string(),
                index_type: Some(description.index_type().to_string()),
                type_url: Some(description.type_url().to_string()),
                num_indexed_rows: Some(description.rows_indexed() as i64),
                num_unindexed_rows: Some(
                    total_rows.saturating_sub(description.rows_indexed()) as i64
                ),
                size_bytes: description.total_size_bytes().map(|size| size as i64),
                num_segments: Some(saturating_i32(segments.len())),
                created_at: segments
                    .iter()
                    .filter_map(|segment| segment.created_at)
                    .min()
                    .map(|ts| ts.to_rfc3339()),
                index_version: segments.first().map(|segment| segment.index_version),
                index_details: description.details().ok(),
            });
        }
        indexes.sort_by(|a, b| a.index_name.cmp(&b.index_name));
        let page_token = paginate_sorted(&mut indexes, request.page_token, request.limit, |i| {
            &i.index_name
        });
        Ok(ListTableIndicesResponse {
            indexes,
            page_token,
            ..Default::default()
        })
    }

    /// Mirrors upstream `DirectoryNamespace::describe_table_index_stats`.
    async fn describe_table_index_stats(
        &self,
        request: DescribeTableIndexStatsRequest,
    ) -> Result<DescribeTableIndexStatsResponse> {
        reject_branch(request.branch.as_deref())?;
        let version = parse_version(request.version)?;
        let index_name = request
            .index_name
            .filter(|name| !name.is_empty())
            .ok_or_else(|| invalid_input("index name is required".into()))?;
        let OpenTable {
            location, dataset, ..
        } = self
            .open_table(&request.id.unwrap_or_default(), version)
            .await?;
        let index_not_found = |e: lance::Error| -> Error {
            NamespaceError::TableIndexNotFound {
                message: format!("index {index_name:?} at {location}: {e}"),
            }
            .into()
        };
        let metadatas = dataset
            .load_indices_by_name(&index_name)
            .await
            .map_err(index_not_found)?;
        if metadatas.first().is_some_and(is_system_index) {
            return Err(NamespaceError::Unsupported {
                message: format!("system index {index_name:?} is not exposed"),
            }
            .into());
        }
        let stats = dataset
            .index_statistics(&index_name)
            .await
            .map_err(index_not_found)?;
        let stats: serde_json::Value = serde_json::from_str(&stats)
            .map_err(|e| internal(format!("parse index statistics at {location}: {e}")))?;
        let get_i64 = |key: &str| {
            stats
                .get(key)
                .and_then(|v| v.as_i64().or_else(|| v.as_u64().map(|n| n as i64)))
        };
        let get_str = |key: &str| stats.get(key).and_then(|v| v.as_str()).map(str::to_string);
        Ok(DescribeTableIndexStatsResponse {
            distance_type: get_str("distance_type"),
            index_type: get_str("index_type"),
            num_indexed_rows: get_i64("num_indexed_rows"),
            num_unindexed_rows: get_i64("num_unindexed_rows"),
            num_indices: get_i64("num_indices").and_then(|n| i32::try_from(n).ok()),
            ..Default::default()
        })
    }

    async fn list_table_tags(
        &self,
        request: ListTableTagsRequest,
    ) -> Result<ListTableTagsResponse> {
        let OpenTable {
            location, dataset, ..
        } = self
            .open_table(&request.id.unwrap_or_default(), None)
            .await?;
        let mut tags: Vec<_> = dataset
            .tags()
            .list()
            .await
            .map_err(lance_error(format!("list tags at {location}")))?
            .into_iter()
            .collect();
        tags.sort_by(|a, b| a.0.cmp(&b.0));
        let page_token = paginate_sorted(&mut tags, request.page_token, request.limit, |t| &t.0);
        Ok(ListTableTagsResponse {
            tags: tags
                .into_iter()
                .map(|(name, contents)| {
                    let mut tag =
                        TagContents::new(contents.version as i64, contents.manifest_size as i64);
                    tag.branch = contents.branch;
                    (name, tag)
                })
                .collect(),
            page_token,
            ..Default::default()
        })
    }

    async fn list_table_branches(
        &self,
        request: ListTableBranchesRequest,
    ) -> Result<ListTableBranchesResponse> {
        let OpenTable {
            location, dataset, ..
        } = self
            .open_table(&request.id.unwrap_or_default(), None)
            .await?;
        let mut branches: Vec<_> = dataset
            .list_branches()
            .await
            .map_err(lance_error(format!("list branches at {location}")))?
            .into_iter()
            .collect();
        branches.sort_by(|a, b| a.0.cmp(&b.0));
        let page_token =
            paginate_sorted(&mut branches, request.page_token, request.limit, |b| &b.0);
        Ok(ListTableBranchesResponse {
            branches: branches
                .into_iter()
                .map(|(name, contents)| {
                    let mut branch = BranchContents::new(
                        contents.parent_version as i64,
                        contents.create_at as i64,
                        contents.manifest_size as i64,
                    );
                    branch.parent_branch = contents.parent_branch;
                    branch.metadata = (!contents.metadata.is_empty()).then_some(contents.metadata);
                    (name, branch)
                })
                .collect(),
            page_token,
            ..Default::default()
        })
    }
}

impl PostgresNamespace {
    async fn table_row_exists(&self, namespace: &[String], name: &str) -> Result<bool> {
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM tables WHERE namespace_path = $1 AND name = $2)",
        )
        .bind(namespace)
        .bind(name)
        .fetch_one(&self.pool)
        .await
        .map_err(db_error)
    }
}

/// Exclusive start-after pagination over names sorted by byte order, as DirectoryNamespace does.
struct Page {
    start_after: Option<String>,
    limit: Option<usize>,
}

impl Page {
    fn new(page_token: Option<String>, limit: Option<i32>) -> Self {
        Self {
            start_after: page_token.filter(|t| !t.is_empty()),
            // A negative or missing limit lists everything.
            limit: limit.and_then(|l| usize::try_from(l).ok()),
        }
    }

    /// One extra row tells whether another page exists.
    fn fetch_limit(&self) -> Option<i64> {
        self.limit.map(|l| l as i64 + 1)
    }

    fn finish(&self, mut names: Vec<String>) -> (Vec<String>, Option<String>) {
        match self.limit {
            Some(limit) if names.len() > limit => {
                names.truncate(limit);
                let token = names.last().cloned();
                (names, token)
            }
            _ => (names, None),
        }
    }
}

pub(crate) fn split_table_id(id: &[String]) -> Result<(&[String], &str)> {
    match id.split_last() {
        Some((name, namespace)) => Ok((namespace, name)),
        None => Err(invalid_input("table id must not be empty".into())),
    }
}

fn validate_names(path: &[String]) -> Result<()> {
    for name in path {
        if name.is_empty() || name.contains(DELIMITER) || name.chars().any(char::is_control) {
            return Err(invalid_input(format!(
                "invalid name {name:?}: names must be non-empty without '{DELIMITER}' or control characters"
            )));
        }
    }
    Ok(())
}

/// Spec modes are case-insensitive and accept PascalCase or snake_case.
fn normalize_mode(mode: Option<&str>, default: &str) -> String {
    mode.unwrap_or(default)
        .to_ascii_lowercase()
        .replace('_', "")
}

fn join_id(path: &[String]) -> String {
    path.join(DELIMITER)
}

fn saturating_i32(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

pub(crate) fn invalid_input(message: String) -> Error {
    NamespaceError::InvalidInput { message }.into()
}

fn namespace_not_found(path: &[String]) -> Error {
    NamespaceError::NamespaceNotFound {
        message: join_id(path),
    }
    .into()
}

fn parent_not_found(path: &[String]) -> Error {
    namespace_not_found(&path[..path.len() - 1])
}

fn namespace_not_empty(path: &[String]) -> Error {
    NamespaceError::NamespaceNotEmpty {
        message: join_id(path),
    }
    .into()
}

fn table_not_found(id: &[String]) -> Error {
    NamespaceError::TableNotFound {
        message: join_id(id),
    }
    .into()
}

fn table_already_exists(id: &[String]) -> Error {
    NamespaceError::TableAlreadyExists {
        message: join_id(id),
    }
    .into()
}

fn db_error(e: sqlx::Error) -> Error {
    let message = format!("database error: {e}");
    // Deadlock (40P01) and serialization failure (40001) are retryable conflicts.
    let code = e.as_database_error().and_then(|d| d.code());
    if matches!(code.as_deref(), Some("40P01" | "40001")) {
        NamespaceError::ConcurrentModification { message }.into()
    } else {
        NamespaceError::Internal { message }.into()
    }
}

/// RESTRICT foreign keys enforce parent existence and drop emptiness atomically; map their
/// violation to the caller's error and anything else through `db_error`.
fn on_foreign_key(e: sqlx::Error, error: impl FnOnce() -> Error) -> Error {
    if e.as_database_error()
        .is_some_and(|d| d.is_foreign_key_violation())
    {
        error()
    } else {
        db_error(e)
    }
}

fn parse_version(version: Option<i64>) -> Result<Option<u64>> {
    version
        .map(|v| u64::try_from(v).map_err(|_| invalid_input(format!("invalid version {v}"))))
        .transpose()
}

/// Only the main branch is readable; "main" is accepted as its explicit name.
fn reject_branch(branch: Option<&str>) -> Result<()> {
    match branch {
        None | Some("" | "main") => Ok(()),
        Some(branch) => Err(NamespaceError::Unsupported {
            message: format!("reading branch {branch:?} is not supported"),
        }
        .into()),
    }
}

pub(crate) fn internal(message: String) -> Error {
    NamespaceError::Internal { message }.into()
}

/// Maps a Lance error from reading one version; a missing file is TableVersionNotFound, which
/// [`PostgresNamespace::read_table`] takes as a sign the current version may be gone.
pub(crate) fn lance_error(context: String) -> impl FnOnce(lance::Error) -> Error {
    move |e| {
        let message = format!("{context}: {e}");
        if is_missing(&e) {
            NamespaceError::TableVersionNotFound { message }.into()
        } else {
            internal(message)
        }
    }
}

/// Lance's errors for a missing object, version, or dataset.
pub(crate) fn is_missing(e: &lance::Error) -> bool {
    matches!(
        e.find_root(),
        lance::Error::DatasetNotFound { .. } | lance::Error::VersionNotFound { .. }
    ) || e.is_not_found()
}

fn is_version_missing(e: &Error) -> bool {
    matches!(e, Error::Namespace { source, .. } if source
        .downcast_ref::<NamespaceError>()
        .is_some_and(|e| matches!(e, NamespaceError::TableVersionNotFound { .. })))
}

/// The row changed location or vanished between reading it and updating it.
fn concurrent_registration(id: &[String]) -> Error {
    NamespaceError::ConcurrentModification {
        message: format!(
            "{} was re-registered or deregistered concurrently",
            join_id(id)
        ),
    }
    .into()
}

/// A session whose metadata cache gets three quarters of `budget_bytes` and index cache the
/// rest; Lance's own defaults (1 GiB and 6 GiB) exceed the service's instance memory.
fn session(budget_bytes: usize) -> Arc<Session> {
    let metadata = budget_bytes / 4 * 3;
    Arc::new(Session::new(
        budget_bytes - metadata,
        metadata,
        Arc::new(ObjectStoreRegistry::default()),
    ))
}

fn table_version(dataset: &Dataset) -> TableVersion {
    let location = dataset.manifest_location();
    let version = dataset.version();
    TableVersion {
        version: version.version as i64,
        manifest_path: location.path.to_string(),
        manifest_size: location.size.map(|size| size as i64),
        e_tag: location.e_tag.clone(),
        timestamp_millis: Some(version.timestamp.timestamp_millis()),
        metadata: Some(version.metadata.into_iter().collect()),
    }
}

/// Start-after pagination over items sorted by `key`; returns the next page token.
fn paginate_sorted<T>(
    items: &mut Vec<T>,
    page_token: Option<String>,
    limit: Option<i32>,
    key: impl Fn(&T) -> &String,
) -> Option<String> {
    if let Some(after) = page_token.filter(|t| !t.is_empty()) {
        items.retain(|item| *key(item) > after);
    }
    match limit.and_then(|l| usize::try_from(l).ok()) {
        Some(limit) if items.len() > limit => {
            items.truncate(limit);
            items.last().map(|item| key(item).clone())
        }
        _ => None,
    }
}

/// Index into [`STORAGE_CATEGORIES`] for a path relative to the dataset location.
fn storage_category(relative: &str) -> usize {
    let category = match relative.split_once('/') {
        Some(("data", file)) if file.ends_with(".lance") => "data",
        Some(("data", file)) if file.ends_with(".blob") => "blob",
        Some(("_indices", _)) => "index",
        Some(("_deletions", _)) => "deletion",
        Some(("_versions", _)) => "manifest",
        Some(("_transactions", _)) => "transaction",
        _ => "other",
    };
    STORAGE_CATEGORIES
        .iter()
        .position(|&name| name == category)
        .expect("category is listed")
}

/// Paths, relative to the dataset location, of the objects one version references: its data
/// files and their sidecar blob files, deletion files, index directories, manifest, and
/// external transaction file. Files under other bases are not under the location and are left
/// out.
struct VersionFiles {
    files: HashSet<String>,
    /// Index UUIDs; each index's files live under `_indices/<uuid>/`.
    indices: HashSet<String>,
}

impl VersionFiles {
    async fn of(dataset: &Dataset) -> lance::Result<Self> {
        let manifest = dataset.manifest();
        let mut files = HashSet::new();
        for fragment in manifest.fragments.iter() {
            for file in fragment.files.iter().filter(|f| f.base_id.is_none()) {
                files.insert(format!("data/{}", file.path));
            }
            if let Some(deletion) = &fragment.deletion_file {
                files.insert(relative_deletion_file_path(fragment.id, deletion));
            }
        }
        let base = dataset.branch_location().path;
        let manifest_path = dataset.manifest_location().path.as_ref();
        files.insert(
            manifest_path
                .strip_prefix(base.as_ref())
                .map_or(manifest_path, |p| p.trim_start_matches('/'))
                .to_string(),
        );
        if let Some(transaction) = manifest
            .transaction_file
            .as_deref()
            .filter(|t| !t.is_empty())
        {
            files.insert(format!("_transactions/{transaction}"));
        }
        let indices = dataset
            .load_indices()
            .await?
            .iter()
            .map(|index| index.uuid.to_string())
            .collect();
        Ok(Self { files, indices })
    }

    fn contains(&self, relative: &str) -> bool {
        if self.files.contains(relative) {
            return true;
        }
        // Sidecar blobs of data file `data/<key>.lance` live at `data/<key>/<id>.blob`.
        if let Some((key, _)) = relative
            .strip_prefix("data/")
            .and_then(|rest| rest.split_once('/'))
            .filter(|(_, file)| file.ends_with(".blob"))
        {
            return self.files.contains(&format!("data/{key}.lance"));
        }
        relative
            .strip_prefix("_indices/")
            .and_then(|rest| rest.split_once('/'))
            .is_some_and(|(uuid, _)| self.indices.contains(uuid))
    }
}
