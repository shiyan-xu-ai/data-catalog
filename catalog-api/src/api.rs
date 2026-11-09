//! Public REST API: a hand-written subset of the Lance Namespace basic op set plus a
//! catalog-defined `/ext` surface for enriched listing and version-level metadata.
//!
//! ## State model
//!
//! Reads serve the merged catalog view from [`crate::catalog::Catalog`] (the derived snapshot,
//! written by the sync, with the authored overlay applied). Mutations write only the authored
//! overlay ([`catalog_store::MetaStore`]) via object-store conditional writes (ETag CAS) — there
//! is no leader and no lock, because the derived snapshot is sync-written last-wins and the
//! authored overlay is serialized per-table by S3 itself. Any instance can mutate. After a
//! mutation the read cache is invalidated so a subsequent read on the same instance sees it.
//!
//! The catalog is a curated allowlist: `DeclareTable` registers a table (writes its overlay),
//! and only registered tables are synced, so declaring is what makes a table's derived fields
//! appear on the next sync.
//! `DeregisterTable` clears the overlay, removing the table from the registered set — the next
//! sync then drops its derived entry from the snapshot.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{any, get};
use axum::{Json, Router};
use catalog_core::{TableEntry, TableVersion};
use catalog_store::{MetaStore, SyncConfig};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::catalog::Catalog;
use crate::catalog_config::CatalogConfig;

/// Numeric error codes adopted from the Namespace spec where they overlap with what this service
/// implements. `error_code` 0 marks a non-spec operational/internal error.
mod error_code {
    pub const NAMESPACE_NOT_FOUND: i32 = 1;
    pub const TABLE_NOT_FOUND: i32 = 4;
    pub const TABLE_VERSION_NOT_FOUND: i32 = 11;
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error_code: i32,
    message: String,
}

fn error(
    status: StatusCode,
    code: i32,
    message: impl Into<String>,
) -> (StatusCode, Json<ErrorResponse>) {
    (
        status,
        Json(ErrorResponse {
            error_code: code,
            message: message.into(),
        }),
    )
}

fn table_not_found(id: &str) -> (StatusCode, Json<ErrorResponse>) {
    error(
        StatusCode::NOT_FOUND,
        error_code::TABLE_NOT_FOUND,
        format!("table not found: {id}"),
    )
}

/// Map an internal failure to a 500. The underlying error (which may carry object-store paths or
/// other internal detail) is logged, not returned to the client.
fn internal_error(e: anyhow::Error) -> (StatusCode, Json<ErrorResponse>) {
    tracing::error!(error = %e, "request failed with an internal error");
    error(
        StatusCode::INTERNAL_SERVER_ERROR,
        0,
        "internal server error",
    )
}

/// Shared state for the public REST API routes.
#[derive(Clone)]
pub struct ApiState {
    /// Merged read model (derived snapshot + authored overlay), lazily revalidated.
    pub catalog: Arc<Catalog>,
    /// Registration and owner metadata store, mutated via ETag CAS.
    pub meta: MetaStore,
    /// Object-store access for auxiliary sample reads.
    pub sync_cfg: SyncConfig,
    /// Deployment region and registered bucket/namespaces used to validate registration.
    pub catalog_cfg: Arc<CatalogConfig>,
}

// ---------------------------------------------------------------------------
// Basic ops
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, PartialEq, Eq, PartialOrd, Ord)]
struct NamespaceRef {
    bucket: String,
    namespace: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ListNamespacesResponse {
    namespaces: Vec<NamespaceRef>,
}

/// `ListNamespaces` -- distinct `(bucket, namespace)` pairs present on cataloged tables. Each
/// entry carries its bucket so the id `describe_namespace` expects (`bucket:prefix[:prefix...]`)
/// can be reconstructed by callers.
async fn list_namespaces(State(state): State<ApiState>) -> Json<ListNamespacesResponse> {
    let view = state.catalog.view().await;
    let mut namespaces: Vec<NamespaceRef> = view
        .iter()
        .map(|e| NamespaceRef {
            bucket: e.bucket.clone(),
            namespace: e.namespace.segments().to_vec(),
        })
        .collect();
    namespaces.sort();
    namespaces.dedup();
    Json(ListNamespacesResponse { namespaces })
}

#[derive(Debug, Serialize)]
struct DescribeNamespaceResponse {
    namespace: Vec<String>,
    table_count: usize,
}

/// `DescribeNamespace` -- `id` is `bucket:prefix[:prefix...]`: the first `:`-separated segment is
/// the bucket, the rest is the namespace prefix. 404 if no table references it.
async fn describe_namespace(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Json<DescribeNamespaceResponse>, (StatusCode, Json<ErrorResponse>)> {
    let mut segments = id.split(catalog_core::ID_SEP);
    let bucket = segments.next().unwrap_or_default().to_string();
    let prefix: Vec<String> = segments.map(str::to_string).collect();
    let view = state.catalog.view().await;
    let table_count = view
        .iter()
        .filter(|e| e.bucket == bucket && e.namespace.segments() == prefix.as_slice())
        .count();
    if table_count == 0 {
        return Err(error(
            StatusCode::NOT_FOUND,
            error_code::NAMESPACE_NOT_FOUND,
            format!("namespace not found: {id}"),
        ));
    }
    Ok(Json(DescribeNamespaceResponse {
        namespace: prefix,
        table_count,
    }))
}

#[derive(Debug, Serialize)]
struct ListTablesResponse {
    tables: Vec<String>,
}

/// `ListTables` -- names only.
async fn list_tables(State(state): State<ApiState>) -> Json<ListTablesResponse> {
    let view = state.catalog.view().await;
    Json(ListTablesResponse {
        tables: view.iter().map(|e| e.id.clone()).collect(),
    })
}

/// `DescribeTable` -- full merged detail.
async fn describe_table(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Json<TableEntry>, (StatusCode, Json<ErrorResponse>)> {
    let view = state.catalog.view().await;
    view.iter()
        .find(|e| e.id == id)
        .cloned()
        .map(Json)
        .ok_or_else(|| table_not_found(&id))
}

#[derive(Debug, Default, Deserialize)]
struct DeclareTableRequest {
    /// `None` (omitted) leaves the existing owner unchanged; `Some` sets it.
    owner: Option<String>,
}

/// `DeclareTable` -- registers a table by writing its authored overlay (creating it if absent),
/// optionally setting `owner`. Registration is what puts the table in the synced set,
/// so its derived fields (versions, sizes, ...) are filled by the next sync. Idempotent; any
/// instance serves it.
///
/// The body is optional: a bare `PUT /v1/table/:id` (no body / no `content-type`) just registers
/// the table with no owner. A present-but-malformed JSON body is a 400. The id must parse
/// as a composite id (`catalog_core::parse_table_id`) whose `(region, bucket, namespace)` matches
/// this deployment's `CatalogConfig` -- the region equals `catalog_cfg.region` and the bucket's
/// namespace is registered -- otherwise 400. This is what keeps the catalog scoped to only the
/// buckets/namespaces this deployment is configured for.
async fn declare_table(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    body: axum::body::Bytes,
) -> Result<Json<TableEntry>, (StatusCode, Json<ErrorResponse>)> {
    let parsed = catalog_core::parse_table_id(&id).ok_or_else(|| {
        error(
            StatusCode::BAD_REQUEST,
            0,
            format!("malformed table id: {id}"),
        )
    })?;
    if parsed.region != state.catalog_cfg.region
        || !state
            .catalog_cfg
            .namespace_registered(&parsed.bucket, &parsed.namespace)
    {
        return Err(error(
            StatusCode::BAD_REQUEST,
            0,
            format!("id {id} is not under a registered namespace"),
        ));
    }
    let req: DeclareTableRequest = if body.is_empty() {
        DeclareTableRequest::default()
    } else {
        serde_json::from_slice(&body).map_err(|e| {
            error(
                StatusCode::BAD_REQUEST,
                0,
                format!("invalid JSON body: {e}"),
            )
        })?
    };
    state
        .meta
        .mutate_meta(&id, |m| {
            if let Some(owner) = &req.owner {
                m.owner = Some(owner.clone());
            }
        })
        .await
        .map_err(internal_error)?;
    state.catalog.invalidate().await;

    let view = state.catalog.view().await;
    view.into_iter()
        .find(|e| e.id == id)
        .map(Json)
        .ok_or_else(|| table_not_found(&id))
}

/// `DeregisterTable` -- unregisters a table by deleting its authored overlay, removing it from
/// the synced set. 404 if the table is not cataloged at all. The derived entry lingers in the
/// snapshot until the next sync, which (no longer seeing it registered) drops it — so a
/// deregistered table disappears from the catalog within one sync interval.
async fn deregister_table(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let present = state.catalog.view().await.iter().any(|e| e.id == id);
    if !present {
        return Err(table_not_found(&id));
    }
    state.meta.delete_meta(&id).await.map_err(internal_error)?;
    state.catalog.invalidate().await;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// /ext extension surface
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ExpandQuery {
    #[allow(dead_code)]
    expand: Option<String>,
}

/// `GET /ext/v1/tables` -- enriched listing (full merged detail; `expand` accepted, unused).
async fn ext_list_tables(
    State(state): State<ApiState>,
    Query(_expand): Query<ExpandQuery>,
) -> Json<Vec<TableEntry>> {
    Json(state.catalog.view().await)
}

/// `GET /ext/v1/tables/{id}/versions/{vid}` -- single version detail.
async fn ext_get_version(
    State(state): State<ApiState>,
    Path((id, vid)): Path<(String, String)>,
) -> Result<Json<TableVersion>, (StatusCode, Json<ErrorResponse>)> {
    let view = state.catalog.view().await;
    let table = view
        .iter()
        .find(|e| e.id == id)
        .ok_or_else(|| table_not_found(&id))?;
    table
        .versions
        .iter()
        .find(|v| v.version_id == vid)
        .cloned()
        .map(Json)
        .ok_or_else(|| {
            error(
                StatusCode::NOT_FOUND,
                error_code::TABLE_VERSION_NOT_FOUND,
                format!("version not found: {id}/{vid}"),
            )
        })
}

#[derive(Debug, Deserialize)]
struct SampleQuery {
    /// Aux entry name within the version (may contain `/` for nested sidecars).
    name: String,
    limit: Option<usize>,
}

#[derive(Debug, Serialize)]
struct SampleResponse {
    table_id: String,
    version_id: String,
    aux_name: String,
    format: catalog_core::AuxFormat,
    schema: Vec<serde_json::Value>,
    rows: Vec<serde_json::Value>,
}

/// `GET /ext/v1/tables/{id}/versions/{vid}/aux/sample?name=&limit=` — read up to `limit`
/// (default 20, max 100) rows from an auxiliary table. Lance aux scans its `dataset_path`;
/// parquet aux dirs scan via DataFusion. Read-only; 30s bound.
async fn ext_sample_aux(
    State(state): State<ApiState>,
    Path((id, vid)): Path<(String, String)>,
    Query(q): Query<SampleQuery>,
) -> Result<Json<SampleResponse>, (StatusCode, Json<ErrorResponse>)> {
    let view = state.catalog.view().await;
    let table = view
        .iter()
        .find(|e| e.id == id)
        .ok_or_else(|| table_not_found(&id))?;
    let version = table
        .versions
        .iter()
        .find(|v| v.version_id == vid)
        .ok_or_else(|| {
            error(
                StatusCode::NOT_FOUND,
                error_code::TABLE_VERSION_NOT_FOUND,
                format!("version not found: {id}/{vid}"),
            )
        })?;
    let entry = version
        .aux
        .iter()
        .find(|a| a.name == q.name)
        .ok_or_else(|| {
            error(
                StatusCode::NOT_FOUND,
                0,
                format!("aux not found on {id}/{vid}: {}", q.name),
            )
        })?;

    let limit = q
        .limit
        .unwrap_or(crate::sample::DEFAULT_SAMPLE_ROWS)
        .clamp(1, crate::sample::MAX_SAMPLE_ROWS);
    const SAMPLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
    let sampled = match entry.format {
        catalog_core::AuxFormat::Lance => {
            let uri = entry.dataset_path.as_deref().unwrap_or(&entry.path);
            tokio::time::timeout(SAMPLE_TIMEOUT, crate::sample::sample_lance(uri, limit)).await
        }
        catalog_core::AuxFormat::Parquet => {
            tokio::time::timeout(
                SAMPLE_TIMEOUT,
                crate::sample::sample_parquet(&state.sync_cfg, &entry.path, limit),
            )
            .await
        }
        other => {
            return Err(error(
                StatusCode::BAD_REQUEST,
                0,
                format!("sampling is not supported for {other:?} aux"),
            ))
        }
    };
    let sample = sampled
        .map_err(|_| anyhow::anyhow!("aux sample timed out after {SAMPLE_TIMEOUT:?}"))
        .and_then(|r| r)
        .map_err(internal_error)?;
    Ok(Json(SampleResponse {
        table_id: id,
        version_id: vid,
        aux_name: q.name,
        format: entry.format,
        schema: sample.schema,
        rows: sample.rows,
    }))
}

/// Build the public `/v1` + `/ext/v1` router wired to `state`.
pub fn api_router(state: ApiState) -> Router {
    Router::new()
        .route("/v1/namespaces", get(list_namespaces))
        .route("/v1/namespaces/:id", get(describe_namespace))
        .route("/v1/tables", get(list_tables))
        .route(
            "/v1/table/:id",
            get(describe_table)
                .put(declare_table)
                .delete(deregister_table),
        )
        .route("/ext/v1/tables", get(ext_list_tables))
        .route("/ext/v1/tables/:id/versions/:vid", get(ext_get_version))
        .route(
            "/ext/v1/tables/:id/versions/:vid/aux/sample",
            get(ext_sample_aux),
        )
        .route("/v1/*path", any(|| async { StatusCode::NOT_FOUND }))
        .route("/ext/*path", any(|| async { StatusCode::NOT_FOUND }))
        .with_state(state)
}
