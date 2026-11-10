use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use lance::dataset::builder::DatasetBuilder;
use lance::session::Session;
use lance::Dataset;
use lance_namespace::models::{
    CreateNamespaceRequest, CreateNamespaceResponse, DeregisterTableRequest,
    DeregisterTableResponse, DescribeNamespaceRequest, DescribeNamespaceResponse,
    DescribeTableRequest, DescribeTableResponse, DropNamespaceRequest, DropNamespaceResponse,
    ListNamespacesRequest, ListNamespacesResponse, ListTablesRequest, ListTablesResponse,
    NamespaceExistsRequest, RegisterTableRequest, RegisterTableResponse, TableBasicStats,
    TableExistsRequest,
};
use lance_namespace::schema::arrow_schema_to_json;
use lance_namespace::{Error, LanceNamespace, NamespaceError, Result};
use sqlx::types::Json;
use sqlx::PgPool;

/// Path delimiter of REST ids. Names may not contain it, so joined ids stay unambiguous.
const DELIMITER: &str = "$";

type Properties = HashMap<String, String>;

/// A [`LanceNamespace`] whose namespaces and table registrations live in Postgres.
///
/// Postgres stores identity, location, and properties only; version, schema, and stats are read
/// from the Lance manifest when detailed metadata is requested. `storage_options` are used to
/// open datasets server-side and are never returned to callers.
pub struct PostgresNamespace {
    pool: PgPool,
    storage_options: HashMap<String, String>,
    session: Arc<Session>,
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
            session: Arc::new(Session::default()),
        }
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
        let version = request
            .version
            .map(|v| u64::try_from(v).map_err(|_| invalid_input(format!("invalid version {v}"))))
            .transpose()?;
        let id = request.id.unwrap_or_default();
        let (namespace, name) = split_table_id(&id)?;
        let row: Option<(String, Json<Properties>)> = sqlx::query_as(
            "SELECT location, properties FROM tables WHERE namespace_path = $1 AND name = $2",
        )
        .bind(namespace)
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(db_error)?;
        let (location, Json(properties)) = row.ok_or_else(|| table_not_found(&id))?;

        let mut response = DescribeTableResponse {
            table: Some(name.to_string()),
            namespace: Some(namespace.to_vec()),
            location: Some(location.clone()),
            table_uri: Some(location.clone()),
            properties: Some(properties),
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

        let dataset = self.open_dataset(&location, version).await.map_err(|e| {
            let message = format!("cannot open Lance dataset at {location}: {e}");
            let missing = matches!(
                e,
                lance::Error::DatasetNotFound { .. }
                    | lance::Error::NotFound { .. }
                    | lance::Error::VersionNotFound { .. }
            );
            match (missing, version) {
                (true, Some(_)) => NamespaceError::TableVersionNotFound { message },
                (true, None) => NamespaceError::TableNotFound { message },
                (false, _) => NamespaceError::Internal { message },
            }
        })?;
        let arrow_schema: arrow_schema::Schema = dataset.schema().into();
        let num_deleted_rows =
            dataset
                .count_deleted_rows()
                .await
                .map_err(|e| NamespaceError::Internal {
                    message: format!("count deleted rows at {location}: {e}"),
                })?;
        response.version = Some(dataset.version().version as i64);
        response.schema = Some(Box::new(arrow_schema_to_json(&arrow_schema)?));
        response.stats = Some(Box::new(TableBasicStats::new(
            saturating_i32(num_deleted_rows),
            saturating_i32(dataset.count_fragments()),
        )));
        Ok(response)
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
        self.open_dataset(&request.location, None)
            .await
            .map_err(|e| {
                invalid_input(format!(
                    "cannot open Lance dataset at {}: {e}",
                    request.location
                ))
            })?;

        let properties = Json(request.properties.unwrap_or_default());
        let sql = if overwrite {
            "INSERT INTO tables (namespace_path, name, location, properties) VALUES ($1, $2, $3, $4)
             ON CONFLICT (namespace_path, name)
             DO UPDATE SET location = EXCLUDED.location, properties = EXCLUDED.properties"
        } else {
            "INSERT INTO tables (namespace_path, name, location, properties) VALUES ($1, $2, $3, $4)"
        };
        sqlx::query(sql)
            .bind(namespace)
            .bind(name)
            .bind(&request.location)
            .bind(&properties)
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
            properties: Some(properties.0),
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
        let row: Option<(String, Json<Properties>)> = sqlx::query_as(
            "DELETE FROM tables WHERE namespace_path = $1 AND name = $2
             RETURNING location, properties",
        )
        .bind(namespace)
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(db_error)?;
        let (location, Json(properties)) = row.ok_or_else(|| table_not_found(&id))?;
        Ok(DeregisterTableResponse {
            id: Some(id),
            location: Some(location),
            properties: Some(properties),
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

fn split_table_id(id: &[String]) -> Result<(&[String], &str)> {
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

fn invalid_input(message: String) -> Error {
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
