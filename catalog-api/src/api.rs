//! Public REST API: a small hand-written subset of the Lance Namespace basic op set
//! (design.md §4.1) plus a catalog-defined `/ext` surface for enriched listing and
//! version-level metadata (design.md §4.2/§4.4). v1.0.0 does not aim for full spec
//! conformance -- see findings.md ("Full Namespace spec conformance v1.0.0: deferred").
//!
//! ## Design decisions (documented here for the reviewer; see also progress.md)
//!
//! - **Namespaces are a derived view, not stored state.** A namespace is just the distinct
//!   set of `namespace` values already present on table entries in the registry -- there is
//!   no separate namespace table/storage. `CreateNamespace`/`DropNamespace` are therefore
//!   omitted entirely (a namespace "exists" iff at least one table references it; nothing to
//!   create or drop). `ListNamespaces`/`DescribeNamespace` are read-only derived views.
//! - **All GET/read endpoints serve from the in-memory registry cache** (`RegistryCache`,
//!   populated by the Phase 4 refresh loop), never from storage directly -- this is true of
//!   every pod, leader or not, per the existing read-path architecture.
//! - **Mutations (`DeclareTable`, `DeregisterTable`, version `protect`) are leader-only.** A
//!   non-leader pod returns `503 Service Unavailable` with a short "not leader" body instead
//!   of forwarding the request to the leader -- no cross-pod forwarding in v1.0.0. Clients are
//!   expected to retry (any pod behind the same Service will eventually hit the leader, or the
//!   client can be pointed at a leader-only internal address in a later phase).
//! - **`DeclareTable` on an existing table is an idempotent update, not an error** -- setting
//!   owner/ttl_policy on a table that already exists just updates those fields; `owner`/
//!   `ttl_policy` are only overwritten when the request body actually sets them (omitted =
//!   `null` in JSON = left unchanged; there is no way to explicitly *clear* a previously-set
//!   value back to `null` in v1.0.0). `TableAlreadyExists` (error code 5) is therefore never
//!   raised by this endpoint and is not implemented.
//! - **`DeclareTable` on an unknown table id creates a stub entry** (empty `namespace`, empty
//!   `root_location`, no versions) with `owner`/`ttl_policy` set from the request -- the next
//!   sweep pass fills in `namespace`/`root_location`/`versions` once it observes the table on
//!   S3 (via `apply_sweep_result`'s existing "never clobber owner/ttl_policy" merge rule).
//! - **Every mutation writes straight to `_catalog/registry`** (the same Lance path the sweep
//!   loop writes) and then immediately re-populates the shared `RegistryCache` in-process, so
//!   the change is visible to a subsequent read on the same pod without waiting for the
//!   periodic refresh tick. A `tokio::sync::Mutex` (`ApiState::write_lock`) serializes
//!   concurrent API mutations against each other (read-modify-write is not otherwise atomic).
//!   It does **not** serialize against the independent periodic sweep-loop write -- that
//!   read-modify-write race (API mutation vs. concurrent sweep write) is the same class of
//!   registry-write hazard already carried forward in status.md's Open items (no
//!   storage-layer CAS backstop); not solved in this phase.
//! - **`/ext/v1/tables` ignores `expand` and always returns full detail, with no pagination.**
//!   At v1.0.0's admin scale (dozens of tables, hundreds of versions each, served out of an
//!   in-memory cache) there is no cost problem that expand-filtering or paging would solve;
//!   the `expand` query param is accepted (so callers that pass it don't 400) but has no
//!   effect. Revisit if/when the registry grows large enough for payload size to matter.
//! - **`GET /metrics` is a placeholder.** Phase 7 (monitoring) adds the real Prometheus
//!   text-format metrics (RED, sweep, freshness, s3, is_leader, ttl_*); this phase only wires
//!   the route so it exists and returns valid (if trivial) Prometheus exposition text instead
//!   of 404ing.
//! - **CORS is permissive (`Any` origin, GET/PUT/DELETE) for all `/v1` and `/ext` routes.**
//!   v1.0.0 has no deployed frontend yet, so there's no concrete origin to allow-list. TODO
//!   (Phase 9/10): tighten to the frontend's actual deployed origin once it exists.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, put};
use axum::{Json, Router};
use catalog_core::{TableEntry, TableVersion, TtlPolicy};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex as AsyncMutex;

use crate::leader::{is_leader, LeaderState};
use crate::registry_cache::RegistryCache;

/// Numeric error codes, adopted verbatim from design.md §4.1 where they overlap with what
/// v1.0.0 actually implements. Codes not listed here (e.g. `TableAlreadyExists=5`,
/// `NamespaceNotEmpty=3`) are not raised by any v1.0.0 endpoint -- see module docs.
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

#[derive(Debug, Serialize)]
struct NotLeaderResponse {
    message: &'static str,
}

/// Standard response for a mutation attempted on a non-leader pod: 503, no cross-pod
/// forwarding in v1.0.0 (see module docs).
fn not_leader() -> (StatusCode, Json<NotLeaderResponse>) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(NotLeaderResponse {
            message:
                "this pod is not the current leader; retry (a leader pod will accept the write)",
        }),
    )
}

/// Shared state for the public REST API routes.
#[derive(Clone)]
pub struct ApiState {
    pub registry_path: String,
    pub cache: RegistryCache,
    pub leader_state: LeaderState,
    /// Serializes concurrent API-driven registry mutations against each other. Does not
    /// serialize against the independent sweep-loop write -- see module docs.
    pub write_lock: Arc<AsyncMutex<()>>,
}

impl ApiState {
    pub fn new(registry_path: String, cache: RegistryCache, leader_state: LeaderState) -> Self {
        Self {
            registry_path,
            cache,
            leader_state,
            write_lock: Arc::new(AsyncMutex::new(())),
        }
    }
}

async fn load_registry_map(path: &str) -> HashMap<String, TableEntry> {
    match catalog_core::read_registry(path).await {
        Ok(entries) => entries.into_iter().map(|e| (e.id.clone(), e)).collect(),
        Err(_) => HashMap::new(),
    }
}

/// Write the merged map back to storage and immediately refresh the in-process cache, so a
/// subsequent read on this pod sees the change without waiting for the periodic refresh tick.
async fn persist_and_refresh_cache(
    state: &ApiState,
    map: HashMap<String, TableEntry>,
) -> anyhow::Result<Vec<TableEntry>> {
    let merged: Vec<TableEntry> = map.into_values().collect();
    catalog_core::write_registry(&state.registry_path, &merged).await?;
    *state.cache.write().await = merged.clone();
    Ok(merged)
}

// ---------------------------------------------------------------------------
// Basic ops (design.md §4.1, hand-written v1.0.0 subset)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct ListNamespacesResponse {
    namespaces: Vec<Vec<String>>,
}

/// `ListNamespaces` -- derived from the distinct `namespace` values present on registered
/// tables (see module docs: namespaces are not separately stored).
async fn list_namespaces(State(state): State<ApiState>) -> Json<ListNamespacesResponse> {
    let cache = state.cache.read().await;
    let mut namespaces: Vec<Vec<String>> = cache
        .iter()
        .map(|e| e.namespace.segments().to_vec())
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

/// `DescribeNamespace` -- `id` is a `.`-joined namespace path (e.g. `scenario_dataset_export`
/// or `a.b` for a multi-segment namespace). 404 `NamespaceNotFound` if no table references it.
async fn describe_namespace(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Json<DescribeNamespaceResponse>, (StatusCode, Json<ErrorResponse>)> {
    let segments: Vec<String> = id.split('.').map(str::to_string).collect();
    let cache = state.cache.read().await;
    let table_count = cache
        .iter()
        .filter(|e| e.namespace.segments() == segments.as_slice())
        .count();
    if table_count == 0 {
        return Err(error(
            StatusCode::NOT_FOUND,
            error_code::NAMESPACE_NOT_FOUND,
            format!("namespace not found: {id}"),
        ));
    }
    Ok(Json(DescribeNamespaceResponse {
        namespace: segments,
        table_count,
    }))
}

#[derive(Debug, Serialize)]
struct ListTablesResponse {
    tables: Vec<String>,
}

/// `ListTables` -- names-only per design.md §4.2 ("`ListTablesResponse` carries exactly a set
/// of full identifier strings"). Enriched listing lives at `/ext/v1/tables`.
async fn list_tables(State(state): State<ApiState>) -> Json<ListTablesResponse> {
    let cache = state.cache.read().await;
    let tables = cache.iter().map(|e| e.id.clone()).collect();
    Json(ListTablesResponse { tables })
}

/// `DescribeTable` -- full detail (v1.0.0 does not implement the spec's basic
/// "location-only, no other fields" behavior; the whole cached `TableEntry` is returned,
/// which is strictly more useful and costs nothing extra since it's already in memory).
async fn describe_table(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Json<TableEntry>, (StatusCode, Json<ErrorResponse>)> {
    let cache = state.cache.read().await;
    cache
        .iter()
        .find(|e| e.id == id)
        .cloned()
        .map(Json)
        .ok_or_else(|| table_not_found(&id))
}

#[derive(Debug, Deserialize)]
struct DeclareTableRequest {
    /// `None` (field omitted) leaves the existing owner unchanged; `Some` sets it.
    owner: Option<String>,
    /// `None` (field omitted) leaves the existing ttl_policy unchanged; `Some` sets it.
    ttl_policy: Option<TtlPolicy>,
}

/// `DeclareTable` -- sets `owner`/`ttl_policy` on a table, creating a stub entry if the table
/// hasn't been swept yet. Leader-only write (503 on a non-leader pod). Idempotent: re-declaring
/// an existing table's owner/ttl_policy just updates them, never errors (see module docs).
async fn declare_table(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(req): Json<DeclareTableRequest>,
) -> Result<Json<TableEntry>, axum::response::Response> {
    if !is_leader(&state.leader_state) {
        return Err(not_leader().into_response_pair());
    }

    let _guard = state.write_lock.lock().await;
    let mut map = load_registry_map(&state.registry_path).await;
    let entry = map.entry(id.clone()).or_insert_with(|| TableEntry {
        id: id.clone(),
        name: id.clone(),
        namespace: catalog_core::Namespace::new(Vec::<String>::new()),
        root_location: String::new(),
        owner: None,
        ttl_policy: None,
        last_swept: None,
        versions: Vec::new(),
        aux_latest: Vec::new(),
    });
    if let Some(owner) = req.owner {
        entry.owner = Some(owner);
    }
    if let Some(ttl_policy) = req.ttl_policy {
        entry.ttl_policy = Some(ttl_policy);
    }
    let updated = entry.clone();

    persist_and_refresh_cache(&state, map)
        .await
        .map_err(|e| internal_error(e).into_response_pair())?;

    Ok(Json(updated))
}

/// `DeregisterTable` -- removes a table from the registry. Leader-only write. 404
/// `TableNotFound` if the table doesn't exist.
async fn deregister_table(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<StatusCode, axum::response::Response> {
    if !is_leader(&state.leader_state) {
        return Err(not_leader().into_response_pair());
    }

    let _guard = state.write_lock.lock().await;
    let mut map = load_registry_map(&state.registry_path).await;
    if map.remove(&id).is_none() {
        return Err(table_not_found(&id).into_response_pair());
    }

    persist_and_refresh_cache(&state, map)
        .await
        .map_err(|e| internal_error(e).into_response_pair())?;

    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// /ext extension surface (design.md §4.2/§4.4)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ExpandQuery {
    /// Accepted but unused in v1.0.0 -- see module docs ("always returns full detail").
    #[allow(dead_code)]
    expand: Option<String>,
}

/// `GET /ext/v1/tables?expand=versions,stats,aux` -- enriched listing. v1.0.0 always returns
/// full `TableEntry` detail for every cached table regardless of `expand`, with no pagination
/// (see module docs).
async fn ext_list_tables(
    State(state): State<ApiState>,
    Query(_expand): Query<ExpandQuery>,
) -> Json<Vec<TableEntry>> {
    Json(state.cache.read().await.clone())
}

/// `GET /ext/v1/tables/{id}/versions/{vid}` -- single version detail.
async fn ext_get_version(
    State(state): State<ApiState>,
    Path((id, vid)): Path<(String, String)>,
) -> Result<Json<TableVersion>, (StatusCode, Json<ErrorResponse>)> {
    let cache = state.cache.read().await;
    let table = cache
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
struct ProtectRequest {
    protected: bool,
}

/// `PUT /ext/v1/tables/{id}/versions/{vid}/protect` -- sets the `protected` flag on a version
/// (TTL-exempt). Leader-only write; plumbing built now for the Phase 6 TTL engine to consume.
async fn ext_protect_version(
    State(state): State<ApiState>,
    Path((id, vid)): Path<(String, String)>,
    Json(req): Json<ProtectRequest>,
) -> Result<Json<TableVersion>, axum::response::Response> {
    if !is_leader(&state.leader_state) {
        return Err(not_leader().into_response_pair());
    }

    let _guard = state.write_lock.lock().await;
    let mut map = load_registry_map(&state.registry_path).await;
    let entry = map
        .get_mut(&id)
        .ok_or_else(|| table_not_found(&id).into_response_pair())?;
    let version = entry
        .versions
        .iter_mut()
        .find(|v| v.version_id == vid)
        .ok_or_else(|| {
            error(
                StatusCode::NOT_FOUND,
                error_code::TABLE_VERSION_NOT_FOUND,
                format!("version not found: {id}/{vid}"),
            )
            .into_response_pair()
        })?;
    version.protected = req.protected;
    let updated = version.clone();

    persist_and_refresh_cache(&state, map)
        .await
        .map_err(|e| internal_error(e).into_response_pair())?;

    Ok(Json(updated))
}

fn internal_error(e: anyhow::Error) -> (StatusCode, Json<ErrorResponse>) {
    error(StatusCode::INTERNAL_SERVER_ERROR, 0, e.to_string())
}

/// Small helper trait so handlers returning `Result<_, axum::response::Response>` can convert
/// an `(StatusCode, Json<T>)` pair into a boxed response without repeating `.into_response()`
/// at every call site.
trait IntoResponsePair {
    fn into_response_pair(self) -> axum::response::Response;
}

impl<T: Serialize> IntoResponsePair for (StatusCode, Json<T>) {
    fn into_response_pair(self) -> axum::response::Response {
        use axum::response::IntoResponse;
        self.into_response()
    }
}

// ---------------------------------------------------------------------------
// /metrics placeholder (real metrics land in Phase 7)
// ---------------------------------------------------------------------------

/// Trivial placeholder in valid Prometheus text-exposition format, so the route exists and
/// scraping doesn't 404 before Phase 7 wires the real RED/sweep/freshness/ttl metrics.
async fn metrics() -> (StatusCode, [(&'static str, &'static str); 1], &'static str) {
    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4")],
        "# HELP catalog_api_up Always 1 while the process is serving requests.\n\
         # TYPE catalog_api_up gauge\n\
         catalog_api_up 1\n",
    )
}

/// Build the public `/v1` + `/ext/v1` + `/metrics` router, fully wired to `state` (returns
/// `Router<()>`, ready to `.merge()` into the top-level app router).
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
            "/ext/v1/tables/:id/versions/:vid/protect",
            put(ext_protect_version),
        )
        .route("/metrics", get(metrics))
        .with_state(state)
}
