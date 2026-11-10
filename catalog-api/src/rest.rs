//! Lance REST Catalog routes over a [`LanceNamespace`].
//!
//! Mirrors the upstream `lance-namespace-impls` RestAdapter conventions (route paths, `$` id
//! delimiter, body/route id reconciliation, `ErrorCode` to HTTP status mapping) for the
//! supported operations. The adapter's router is private and binds its own listener, so it
//! cannot share the service port with the UI and health routes. Every other `/v1` route answers
//! the spec's Unsupported error.

use std::sync::Arc;

use axum::body::{to_bytes, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use lance_namespace::models::{
    CreateNamespaceRequest, CreateNamespaceResponse, DeregisterTableRequest,
    DeregisterTableResponse, DescribeNamespaceRequest, DescribeNamespaceResponse,
    DescribeTableRequest, DescribeTableResponse, DropNamespaceRequest, DropNamespaceResponse,
    ErrorResponse, ListNamespacesRequest, ListNamespacesResponse, ListTablesRequest,
    ListTablesResponse, NamespaceExistsRequest, RegisterTableRequest, RegisterTableResponse,
    TableExistsRequest,
};
use lance_namespace::{Error, ErrorCode, LanceNamespace, NamespaceError};
use serde::de::DeserializeOwned;
use serde::Deserialize;

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
        .fallback(unsupported)
        .layer(axum::middleware::map_response(rejection_to_error_response))
        .with_state(backend)
}

#[derive(Deserialize)]
struct PaginationQuery {
    page_token: Option<String>,
    limit: Option<i32>,
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
