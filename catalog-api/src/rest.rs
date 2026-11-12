//! Lance REST Catalog routes over a [`LanceNamespace`], plus the catalog's own `/catalog/v1`
//! routes for facts no spec model carries.
//!
//! Mirrors the upstream `lance-namespace-impls` RestAdapter conventions (route paths, `$` id
//! delimiter, body/route id reconciliation, `ErrorCode` to HTTP status mapping) for the
//! supported operations. The adapter's router is private and binds its own listener, so it
//! cannot share the service port with the UI and health routes. Every other `/v1` route answers
//! the spec's Unsupported error.

use std::sync::Arc;
use std::time::Instant;

use axum::body::{to_bytes, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use catalog_store::{PostgresNamespace, StorageFootprint};
use lance_namespace::models::{
    CreateNamespaceRequest, CreateNamespaceResponse, DeregisterTableRequest,
    DeregisterTableResponse, DescribeNamespaceRequest, DescribeNamespaceResponse,
    DescribeTableIndexStatsRequest, DescribeTableIndexStatsResponse, DescribeTableRequest,
    DescribeTableResponse, DescribeTableVersionRequest, DescribeTableVersionResponse,
    DropNamespaceRequest, DropNamespaceResponse, ErrorResponse, GetTableStatsRequest,
    GetTableStatsResponse, ListNamespacesRequest, ListNamespacesResponse, ListTableBranchesRequest,
    ListTableBranchesResponse, ListTableIndicesRequest, ListTableIndicesResponse,
    ListTableTagsRequest, ListTableTagsResponse, ListTableVersionsRequest,
    ListTableVersionsResponse, ListTablesRequest, ListTablesResponse, NamespaceExistsRequest,
    RegisterTableRequest, RegisterTableResponse, TableExistsRequest,
};
use lance_namespace::{Error, ErrorCode, LanceNamespace, NamespaceError};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Semaphore;

use crate::config::{CatalogConfig, SqlConfig, StoragePricing};

type Backend = Arc<dyn LanceNamespace>;
type ApiResult<T> = Result<T, ApiError>;

const DELIMITER: &str = "$";

/// Router for the `/v1` prefix; nest it at `/v1`.
pub fn router(backend: Backend) -> Router {
    Router::new()
        .route("/namespace/:id/list", get(list_namespaces))
        .route("/namespace/:id/describe", post(describe_namespace))
        .route("/namespace/:id/create", post(create_namespace))
        .route("/namespace/:id/drop", post(drop_namespace))
        .route("/namespace/:id/exists", post(namespace_exists))
        .route("/namespace/:id/table/list", get(list_tables))
        .route("/table", get(list_all_tables))
        .route("/table/:id/register", post(register_table))
        .route("/table/:id/describe", post(describe_table))
        .route("/table/:id/exists", post(table_exists))
        .route("/table/:id/deregister", post(deregister_table))
        .route("/table/:id/stats", post(get_table_stats))
        .route("/table/:id/version/list", post(list_table_versions))
        .route("/table/:id/version/describe", post(describe_table_version))
        .route("/table/:id/index/list", post(list_table_indices))
        .route(
            "/table/:id/index/:index_name/stats",
            post(describe_table_index_stats),
        )
        .route("/table/:id/tags/list", post(list_table_tags))
        .route("/table/:id/branches/list", post(list_table_branches))
        .fallback(unsupported)
        .layer(axum::middleware::map_response(rejection_to_error_response))
        .with_state(backend)
}

/// State of the catalog routes.
#[derive(Clone)]
struct Catalog {
    namespace: Arc<PostgresNamespace>,
    storage_pricing: Arc<StoragePricing>,
    sql: Arc<SqlConfig>,
    /// Per-instance cap on concurrent SQL queries.
    sql_slots: Arc<Semaphore>,
}

/// Router for the catalog's own routes, read-only apart from refresh; nest it at `/catalog/v1`.
pub fn catalog_router(namespace: Arc<PostgresNamespace>, config: CatalogConfig) -> Router {
    Router::new()
        .route("/table/:id/column-storage", get(column_storage))
        .route("/table/:id/storage", get(table_storage))
        .route("/table/:id/timeline", get(table_timeline))
        .route("/table/:id/refresh", post(refresh_table))
        .route("/table/:id/sql", post(run_sql))
        .fallback(unsupported)
        .layer(axum::middleware::map_response(rejection_to_error_response))
        .with_state(Catalog {
            namespace,
            storage_pricing: Arc::new(config.storage_pricing),
            sql_slots: Arc::new(Semaphore::new(config.sql.max_concurrent)),
            sql: Arc::new(config.sql),
        })
}

#[derive(Deserialize)]
struct PaginationQuery {
    page_token: Option<String>,
    limit: Option<i32>,
}

/// Spec list routes take pagination as query parameters; a JSON body is accepted too, and the
/// query wins where both are set.
#[derive(Deserialize)]
struct VersionListQuery {
    page_token: Option<String>,
    limit: Option<i32>,
    descending: Option<bool>,
    branch: Option<String>,
}

#[derive(Deserialize)]
struct DescribeTableQuery {
    with_table_uri: Option<bool>,
    load_detailed_metadata: Option<bool>,
    check_declared: Option<bool>,
}

async fn list_namespaces(
    State(ns): State<Backend>,
    Path(id): Path<String>,
    Query(q): Query<PaginationQuery>,
) -> ApiResult<Json<ListNamespacesResponse>> {
    let request = ListNamespacesRequest {
        id: Some(parse_id(&id)),
        page_token: q.page_token,
        limit: q.limit,
        ..Default::default()
    };
    Ok(Json(ns.list_namespaces(request).await?))
}

async fn describe_namespace(
    State(ns): State<Backend>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<Json<DescribeNamespaceResponse>> {
    let mut request: DescribeNamespaceRequest = parse_body(&body)?;
    reconcile_id(&mut request.id, &id)?;
    Ok(Json(ns.describe_namespace(request).await?))
}

async fn create_namespace(
    State(ns): State<Backend>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<Json<CreateNamespaceResponse>> {
    let mut request: CreateNamespaceRequest = parse_body(&body)?;
    reconcile_id(&mut request.id, &id)?;
    Ok(Json(ns.create_namespace(request).await?))
}

async fn drop_namespace(
    State(ns): State<Backend>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<Json<DropNamespaceResponse>> {
    let mut request: DropNamespaceRequest = parse_body(&body)?;
    reconcile_id(&mut request.id, &id)?;
    Ok(Json(ns.drop_namespace(request).await?))
}

async fn namespace_exists(
    State(ns): State<Backend>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<StatusCode> {
    let mut request: NamespaceExistsRequest = parse_body(&body)?;
    reconcile_id(&mut request.id, &id)?;
    ns.namespace_exists(request).await?;
    Ok(StatusCode::OK)
}

async fn list_tables(
    State(ns): State<Backend>,
    Path(id): Path<String>,
    Query(q): Query<PaginationQuery>,
) -> ApiResult<Json<ListTablesResponse>> {
    let request = ListTablesRequest {
        id: Some(parse_id(&id)),
        page_token: q.page_token,
        limit: q.limit,
        ..Default::default()
    };
    Ok(Json(ns.list_tables(request).await?))
}

async fn list_all_tables(
    State(ns): State<Backend>,
    Query(q): Query<PaginationQuery>,
) -> ApiResult<Json<ListTablesResponse>> {
    let request = ListTablesRequest {
        page_token: q.page_token,
        limit: q.limit,
        ..Default::default()
    };
    Ok(Json(ns.list_all_tables(request).await?))
}

async fn register_table(
    State(ns): State<Backend>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<Json<RegisterTableResponse>> {
    let mut request: RegisterTableRequest = parse_body(&body)?;
    reconcile_id(&mut request.id, &id)?;
    Ok(Json(ns.register_table(request).await?))
}

async fn describe_table(
    State(ns): State<Backend>,
    Path(id): Path<String>,
    Query(q): Query<DescribeTableQuery>,
    body: Bytes,
) -> ApiResult<Json<DescribeTableResponse>> {
    let mut request: DescribeTableRequest = parse_body(&body)?;
    reconcile_id(&mut request.id, &id)?;
    // Query parameters take precedence over the body, as in the upstream adapter.
    request.with_table_uri = q.with_table_uri.or(request.with_table_uri);
    request.load_detailed_metadata = q.load_detailed_metadata.or(request.load_detailed_metadata);
    request.check_declared = q.check_declared.or(request.check_declared);
    Ok(Json(ns.describe_table(request).await?))
}

async fn table_exists(
    State(ns): State<Backend>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<StatusCode> {
    let mut request: TableExistsRequest = parse_body(&body)?;
    reconcile_id(&mut request.id, &id)?;
    ns.table_exists(request).await?;
    Ok(StatusCode::OK)
}

async fn deregister_table(
    State(ns): State<Backend>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<Json<DeregisterTableResponse>> {
    let mut request: DeregisterTableRequest = parse_body(&body)?;
    reconcile_id(&mut request.id, &id)?;
    Ok(Json(ns.deregister_table(request).await?))
}

async fn get_table_stats(
    State(ns): State<Backend>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<Json<GetTableStatsResponse>> {
    let mut request: GetTableStatsRequest = parse_body(&body)?;
    reconcile_id(&mut request.id, &id)?;
    Ok(Json(ns.get_table_stats(request).await?))
}

async fn list_table_versions(
    State(ns): State<Backend>,
    Path(id): Path<String>,
    Query(q): Query<VersionListQuery>,
    body: Bytes,
) -> ApiResult<Json<ListTableVersionsResponse>> {
    let mut request: ListTableVersionsRequest = parse_body(&body)?;
    reconcile_id(&mut request.id, &id)?;
    request.page_token = q.page_token.or(request.page_token);
    request.limit = q.limit.or(request.limit);
    request.descending = q.descending.or(request.descending);
    request.branch = q.branch.or(request.branch);
    Ok(Json(ns.list_table_versions(request).await?))
}

async fn describe_table_version(
    State(ns): State<Backend>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<Json<DescribeTableVersionResponse>> {
    let mut request: DescribeTableVersionRequest = parse_body(&body)?;
    reconcile_id(&mut request.id, &id)?;
    Ok(Json(ns.describe_table_version(request).await?))
}

async fn list_table_indices(
    State(ns): State<Backend>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<Json<ListTableIndicesResponse>> {
    let mut request: ListTableIndicesRequest = parse_body(&body)?;
    reconcile_id(&mut request.id, &id)?;
    Ok(Json(ns.list_table_indices(request).await?))
}

async fn describe_table_index_stats(
    State(ns): State<Backend>,
    Path((id, index_name)): Path<(String, String)>,
    body: Bytes,
) -> ApiResult<Json<DescribeTableIndexStatsResponse>> {
    let mut request: DescribeTableIndexStatsRequest = parse_body(&body)?;
    reconcile_id(&mut request.id, &id)?;
    if request
        .index_name
        .as_ref()
        .is_some_and(|n| *n != index_name)
    {
        return Err(ApiError::invalid_input(format!(
            "body index_name {:?} does not match route index name {index_name:?}",
            request.index_name
        )));
    }
    request.index_name = Some(index_name);
    Ok(Json(ns.describe_table_index_stats(request).await?))
}

async fn list_table_tags(
    State(ns): State<Backend>,
    Path(id): Path<String>,
    Query(q): Query<PaginationQuery>,
    body: Bytes,
) -> ApiResult<Json<ListTableTagsResponse>> {
    let mut request: ListTableTagsRequest = parse_body(&body)?;
    reconcile_id(&mut request.id, &id)?;
    request.page_token = q.page_token.or(request.page_token);
    request.limit = q.limit.or(request.limit);
    Ok(Json(ns.list_table_tags(request).await?))
}

async fn list_table_branches(
    State(ns): State<Backend>,
    Path(id): Path<String>,
    Query(q): Query<PaginationQuery>,
    body: Bytes,
) -> ApiResult<Json<ListTableBranchesResponse>> {
    let mut request: ListTableBranchesRequest = parse_body(&body)?;
    reconcile_id(&mut request.id, &id)?;
    request.page_token = q.page_token.or(request.page_token);
    request.limit = q.limit.or(request.limit);
    Ok(Json(ns.list_table_branches(request).await?))
}

#[derive(Serialize)]
struct ColumnStorageResponse {
    version: u64,
    columns: Vec<ColumnBytesResponse>,
}

#[derive(Serialize)]
struct ColumnBytesResponse {
    name: String,
    bytes_on_disk: u64,
}

async fn column_storage(
    State(catalog): State<Catalog>,
    Path(id): Path<String>,
) -> ApiResult<Json<ColumnStorageResponse>> {
    let storage = catalog.namespace.column_storage(&parse_id(&id)).await?;
    Ok(Json(ColumnStorageResponse {
        version: storage.version,
        columns: storage
            .columns
            .into_iter()
            .map(|c| ColumnBytesResponse {
                name: c.name,
                bytes_on_disk: c.bytes_on_disk,
            })
            .collect(),
    }))
}

#[derive(Serialize)]
struct RefreshResponse {
    previous_version: Option<u64>,
    served_version: u64,
}

async fn refresh_table(
    State(catalog): State<Catalog>,
    Path(id): Path<String>,
) -> ApiResult<Json<RefreshResponse>> {
    let refresh = catalog.namespace.refresh_table(&parse_id(&id)).await?;
    Ok(Json(RefreshResponse {
        previous_version: refresh.previous_version,
        served_version: refresh.served_version,
    }))
}

#[derive(Deserialize)]
struct TimelineQuery {
    page_token: Option<String>,
    limit: Option<usize>,
}

#[derive(Serialize)]
struct TimelineResponse {
    events: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    page_token: Option<String>,
}

async fn table_timeline(
    State(catalog): State<Catalog>,
    Path(id): Path<String>,
    Query(q): Query<TimelineQuery>,
) -> ApiResult<Json<TimelineResponse>> {
    let timeline = catalog
        .namespace
        .timeline(&parse_id(&id), q.page_token.as_deref(), q.limit)
        .await?;
    Ok(Json(TimelineResponse {
        events: timeline.events,
        page_token: timeline.page_token,
    }))
}

#[derive(Serialize)]
struct StorageResponse {
    version: u64,
    current: CurrentStorage,
    on_storage: OnStorage,
    pricing: Pricing,
}

#[derive(Serialize)]
struct CurrentStorage {
    data_bytes: u64,
    index_bytes: u64,
}

#[derive(Serialize)]
struct OnStorage {
    total_bytes: u64,
    total_objects: u64,
    categories: Vec<StorageCategoryResponse>,
}

#[derive(Serialize)]
struct StorageCategoryResponse {
    name: &'static str,
    bytes: u64,
    objects: u64,
}

#[derive(Serialize)]
struct Pricing {
    price_per_gb_month: f64,
    currency: &'static str,
    tier: String,
    estimated_monthly_cost: f64,
}

async fn table_storage(
    State(catalog): State<Catalog>,
    Path(id): Path<String>,
) -> ApiResult<Json<StorageResponse>> {
    let StorageFootprint {
        version,
        data_bytes,
        index_bytes,
        total_bytes,
        total_objects,
        categories,
    } = catalog.namespace.storage_footprint(&parse_id(&id)).await?;
    let pricing = &catalog.storage_pricing;
    Ok(Json(StorageResponse {
        version,
        current: CurrentStorage {
            data_bytes,
            index_bytes,
        },
        on_storage: OnStorage {
            total_bytes,
            total_objects,
            categories: categories
                .into_iter()
                .map(|c| StorageCategoryResponse {
                    name: c.name,
                    bytes: c.bytes,
                    objects: c.objects,
                })
                .collect(),
        },
        pricing: Pricing {
            price_per_gb_month: pricing.price_per_gb_month,
            currency: "USD",
            tier: pricing.tier.clone(),
            // Decimal GB, matching the rate card.
            estimated_monthly_cost: total_bytes as f64 / 1e9 * pricing.price_per_gb_month,
        },
    }))
}

#[derive(Deserialize, Default)]
struct SqlRequest {
    #[serde(default)]
    sql: String,
    /// Version the plain table name resolves to; latest when unset.
    version: Option<u64>,
}

#[derive(Serialize)]
struct SqlResponse {
    version: u64,
    columns: Vec<SqlColumnResponse>,
    rows: Vec<Vec<Value>>,
    row_count: usize,
    truncated: bool,
    elapsed_ms: u64,
}

#[derive(Serialize)]
struct SqlColumnResponse {
    name: String,
    #[serde(rename = "type")]
    data_type: String,
}

async fn run_sql(
    State(catalog): State<Catalog>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<Json<SqlResponse>> {
    let request: SqlRequest = parse_body(&body)?;
    if request.sql.trim().is_empty() {
        return Err(ApiError::invalid_input("sql is required".into()));
    }
    let _slot = catalog.sql_slots.try_acquire().map_err(|_| {
        ApiError::from(Error::from(NamespaceError::Throttling {
            message: format!(
                "too many concurrent SQL queries (limit {}); retry shortly",
                catalog.sql.max_concurrent
            ),
        }))
    })?;
    let started = Instant::now();
    let result = catalog
        .namespace
        .query_sql(
            &parse_id(&id),
            &request.sql,
            request.version,
            &catalog.sql.limits,
        )
        .await?;
    Ok(Json(SqlResponse {
        version: result.version,
        columns: result
            .columns
            .into_iter()
            .map(|c| SqlColumnResponse {
                name: c.name,
                data_type: c.data_type,
            })
            .collect(),
        row_count: result.rows.len(),
        rows: result.rows,
        truncated: result.truncated,
        elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    }))
}

async fn unsupported() -> ApiError {
    ApiError::from(Error::from(NamespaceError::Unsupported {
        message: "operation not supported by this catalog".into(),
    }))
}

/// `$` alone is the root namespace; empty segments are dropped.
fn parse_id(id: &str) -> Vec<String> {
    if id == DELIMITER {
        return Vec::new();
    }
    id.split(DELIMITER)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// An empty body is accepted as the request's defaults.
fn parse_body<T: DeserializeOwned + Default>(body: &Bytes) -> ApiResult<T> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(T::default());
    }
    serde_json::from_slice(body)
        .map_err(|e| ApiError::invalid_input(format!("invalid JSON body: {e}")))
}

/// The route id is authoritative; a body id that disagrees is rejected rather than ignored.
fn reconcile_id(body_id: &mut Option<Vec<String>>, route_id: &str) -> ApiResult<()> {
    let route_id = parse_id(route_id);
    if let Some(body_id) = body_id.as_ref() {
        if *body_id != route_id {
            return Err(ApiError::invalid_input(format!(
                "body id {body_id:?} does not match route id {route_id:?}"
            )));
        }
    }
    *body_id = Some(route_id);
    Ok(())
}

pub struct ApiError(Error);

impl ApiError {
    fn invalid_input(message: String) -> Self {
        Self(NamespaceError::InvalidInput { message }.into())
    }
}

/// Extractor rejections (bad path encoding, query, or body) are plain-text 4xx responses;
/// rewrite them into the spec's InvalidInput error, as the upstream adapter does.
async fn rejection_to_error_response(response: Response) -> Response {
    let rejected = matches!(
        response.status(),
        StatusCode::BAD_REQUEST
            | StatusCode::PAYLOAD_TOO_LARGE
            | StatusCode::UNSUPPORTED_MEDIA_TYPE
            | StatusCode::UNPROCESSABLE_ENTITY
    );
    let is_json = response
        .headers()
        .get(CONTENT_TYPE)
        .is_some_and(|v| v.as_bytes().starts_with(b"application/json"));
    if !rejected || is_json {
        return response;
    }
    let body = to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap_or_default();
    ApiError::invalid_input(String::from_utf8_lossy(&body).into_owned()).into_response()
}

impl From<Error> for ApiError {
    fn from(e: Error) -> Self {
        Self(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let namespace_error = match &self.0 {
            Error::Namespace { source, .. } => source.downcast_ref::<NamespaceError>(),
            _ => None,
        };
        let (code, message) = match namespace_error {
            Some(e) => (e.code(), e.message().to_string()),
            None => (ErrorCode::Internal, self.0.to_string()),
        };
        let status = status_for(code);
        if status.is_server_error() {
            tracing::error!(code = code.as_u32(), error = %message, "request failed");
        }
        let mut body = ErrorResponse::new(code.as_u32() as i32);
        body.error = Some(message);
        (status, Json(body)).into_response()
    }
}

fn status_for(code: ErrorCode) -> StatusCode {
    use ErrorCode::*;
    match code {
        NamespaceNotFound | TableNotFound | TableIndexNotFound | TableTagNotFound
        | TransactionNotFound | TableVersionNotFound | TableColumnNotFound
        | TableBranchNotFound => StatusCode::NOT_FOUND,
        NamespaceAlreadyExists
        | TableAlreadyExists
        | TableIndexAlreadyExists
        | TableTagAlreadyExists
        | TableBranchAlreadyExists
        | ConcurrentModification
        | NamespaceNotEmpty
        | InvalidTableState => StatusCode::CONFLICT,
        InvalidInput | TableSchemaValidationError => StatusCode::BAD_REQUEST,
        Unsupported => StatusCode::NOT_ACCEPTABLE,
        PermissionDenied => StatusCode::FORBIDDEN,
        Unauthenticated => StatusCode::UNAUTHORIZED,
        ServiceUnavailable => StatusCode::SERVICE_UNAVAILABLE,
        Throttling => StatusCode::TOO_MANY_REQUESTS,
        Internal => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

#[cfg(test)]
mod tests {
    use super::parse_id;

    #[test]
    fn parse_id_handles_root_and_nested() {
        assert!(parse_id("$").is_empty());
        assert_eq!(parse_id("prod"), ["prod"]);
        assert_eq!(parse_id("prod$users"), ["prod", "users"]);
        assert_eq!(parse_id("prod$$users"), ["prod", "users"]);
    }
}
