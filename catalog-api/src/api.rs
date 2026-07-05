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
//!   periodic refresh tick. `ApiState::write_lock` is the SAME `RegistryWriteLock` the sweep
//!   loop's `run_sweep_once` holds across its own read-modify-write (see
//!   `crate::registry_lock` module docs) -- so an API mutation's
//!   read-registry/mutate/write-registry critical section can never interleave with either a
//!   concurrent API mutation or the periodic sweep-loop write. This closes the lost-update
//!   race a Phase 5 review found (a sweep reading a stale registry, then overwriting a
//!   concurrently-committed API change) before Phase 6 wires TTL hard-delete to the
//!   `protected` flag, where a silently-reverted `protect` would otherwise be irreversible.
//! - **TTL dry-run/apply** (`GET/POST /ext/v1/tables/{id}/ttl/{dryrun,apply}`) compute
//!   TTL-eligible versions via `catalog_core::ttl_eligible_versions` (per-table `ttl_policy`,
//!   `protected` always exempt, shape gated to `Full`/`LanceOnly`/`SegOnly`). Dry-run is
//!   read-only and leader-independent (served from cache, like other GETs). Apply is
//!   leader-only, recomputes eligibility fresh (never trusts a stale dry-run response),
//!   physically deletes each eligible version's entire `<table>/<timestamp>/` prefix tree via
//!   `catalog_store::delete_prefix`, appends a `TtlAuditRecord` per deletion, and removes the
//!   version from the registry through the same `write_lock`-guarded critical section as
//!   every other mutation. Idempotent: a version already removed from the registry is simply
//!   not recomputed as eligible on the next call, so re-applying is a no-op.
//! - **`/ext/v1/tables` ignores `expand` and always returns full detail, with no pagination.**
//!   At v1.0.0's admin scale (dozens of tables, hundreds of versions each, served out of an
//!   in-memory cache) there is no cost problem that expand-filtering or paging would solve;
//!   the `expand` query param is accepted (so callers that pass it don't 400) but has no
//!   effect. Revisit if/when the registry grows large enough for payload size to matter.
//! - **CORS is permissive (`Any` origin, GET/PUT/DELETE) for all `/v1` and `/ext` routes.**
//!   v1.0.0 has no deployed frontend yet, so there's no concrete origin to allow-list. TODO
//!   (Phase 9/10): tighten to the frontend's actual deployed origin once it exists.
//! - **`GET /metrics` is served on a separate internal port** (default `9090`) by the internal
//!   server in `crate::internal_server`. This allows network policy to restrict Prometheus
//!   scraping to the metrics port without opening it to general API traffic. The main API
//!   router no longer includes a `/metrics` route.

use std::collections::HashMap;
use std::time::Instant;

use axum::extract::{MatchedPath, Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use catalog_core::{TableEntry, TableVersion, TtlAuditRecord, TtlPolicy};
use catalog_store::SweepConfig;
use serde::{Deserialize, Serialize};

use crate::leader::{is_leader, LeaderState};
use crate::metrics;
use crate::registry_cache::RegistryCache;
use crate::registry_lock::RegistryWriteLock;

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

/// Standard response for a mutation attempted on a non-leader pod: 503, no cross-pod
/// forwarding in v1.0.0 (see module docs). Uses the shared `ErrorResponse` envelope so every
/// error this API returns has the same shape; `error_code` 0 marks a non-spec operational error.
fn not_leader() -> (StatusCode, Json<ErrorResponse>) {
    error(
        StatusCode::SERVICE_UNAVAILABLE,
        0,
        "this pod is not the current leader; retry (a leader pod will accept the write)",
    )
}

/// Shared state for the public REST API routes.
#[derive(Clone)]
pub struct ApiState {
    pub registry_path: String,
    pub cache: RegistryCache,
    pub leader_state: LeaderState,
    /// Serializes concurrent registry read-modify-writes against each other -- shared with
    /// the sweep loop's own read-modify-write (see `crate::registry_lock` module docs) so
    /// neither can interleave with the other or with a concurrent API mutation.
    pub write_lock: RegistryWriteLock,
    /// Object-store access for TTL hard-delete (`ttl_apply`), built from the same sweep root
    /// config the sweep loop uses so version `snapshot_path`s resolve to the same storage.
    pub sweep_cfg: SweepConfig,
    /// URI of the `_catalog/ttl_audit` Lance table TTL `apply` appends to.
    pub ttl_audit_path: String,
}

impl ApiState {
    pub fn new(
        registry_path: String,
        cache: RegistryCache,
        leader_state: LeaderState,
        write_lock: RegistryWriteLock,
        sweep_cfg: SweepConfig,
        ttl_audit_path: String,
    ) -> Self {
        Self {
            registry_path,
            cache,
            leader_state,
            write_lock,
            sweep_cfg,
            ttl_audit_path,
        }
    }
}

/// Load the registry into an id-keyed map for a read-modify-write mutation.
///
/// A missing dataset (`Ok(None)`, expected before the first sweep) yields an empty map. A real
/// read error is propagated: mutation handlers turn it into a 500 rather than proceeding, so a
/// transient object-store failure can never make a handler persist a registry rebuilt from an
/// empty map (which `write_registry`'s `Overwrite` would then use to wipe every other table's
/// `owner`/`ttl_policy`/`protected` state).
async fn load_registry_map(path: &str) -> anyhow::Result<HashMap<String, TableEntry>> {
    Ok(catalog_core::read_registry(path)
        .await?
        .unwrap_or_default()
        .into_iter()
        .map(|e| (e.id.clone(), e))
        .collect())
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
    let mut map = load_registry_map(&state.registry_path)
        .await
        .map_err(|e| internal_error(e).into_response_pair())?;
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
    let mut map = load_registry_map(&state.registry_path)
        .await
        .map_err(|e| internal_error(e).into_response_pair())?;
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
    let mut map = load_registry_map(&state.registry_path)
        .await
        .map_err(|e| internal_error(e).into_response_pair())?;
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

// ---------------------------------------------------------------------------
// TTL engine (design.md §5.7 adapted to timestamp-path versions; findings.md's locked
// "hard delete, per-table API policy" semantics)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct TtlDryRunResponse {
    table_id: String,
    /// TTL-eligible version ids under the table's current `ttl_policy`, as of now.
    candidates: Vec<String>,
    /// Sum of `storage_bytes_total` (logical/deduped bytes, see findings.md's Phase 6 carry-
    /// forward note on `storage_bytes_total` vs. physical footprint) over `candidates`.
    reclaimable_bytes: u64,
}

/// `GET /ext/v1/tables/{id}/ttl/dryrun` -- read-only, no leader requirement (any pod can serve
/// this from its registry cache, like any other GET). Computes candidates fresh from the
/// cached state every call; never mutates anything.
async fn ttl_dryrun(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Json<TtlDryRunResponse>, (StatusCode, Json<ErrorResponse>)> {
    let cache = state.cache.read().await;
    let table = cache
        .iter()
        .find(|e| e.id == id)
        .ok_or_else(|| table_not_found(&id))?;
    let policy = table.ttl_policy.unwrap_or_default();
    let eligible =
        catalog_core::ttl_eligible_versions(&policy, &table.versions, chrono::Utc::now());
    let reclaimable_bytes: u64 = eligible.iter().map(|v| v.storage_bytes_total).sum();
    let candidates = eligible.into_iter().map(|v| v.version_id.clone()).collect();
    Ok(Json(TtlDryRunResponse {
        table_id: id,
        candidates,
        reclaimable_bytes,
    }))
}

#[derive(Debug, Serialize)]
struct TtlApplyResponse {
    table_id: String,
    /// Version ids actually hard-deleted by this call.
    deleted: Vec<String>,
    reclaimed_bytes: u64,
}

/// `GET /ext/v1/tables/{id}/ttl/audit` -- read-only audit log for a table. Returns the
/// `TtlAuditRecord`s for `id` (filtered from the global `_catalog/ttl_audit` log). Any pod
/// can serve this; no leader requirement (read-only, like every other GET). Returns an empty
/// list if no TTL deletion has ever been recorded for the table (or if the audit dataset does
/// not exist yet).
async fn ttl_audit(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<TtlAuditRecord>>, (StatusCode, Json<ErrorResponse>)> {
    let records = catalog_core::read_ttl_audit(&state.ttl_audit_path)
        .await
        .map_err(internal_error)?
        .unwrap_or_default();
    let filtered: Vec<TtlAuditRecord> = records.into_iter().filter(|r| r.table_id == id).collect();
    Ok(Json(filtered))
}

/// `POST /ext/v1/tables/{id}/ttl/apply` -- leader-only, IRREVERSIBLE hard delete.
///
/// Recomputes TTL-eligible versions fresh against the current registry state (never trusts a
/// stale dry-run response the caller might be holding). For each eligible version: physically
/// deletes its entire `<table>/<timestamp>/` object-store prefix tree, appends a
/// `TtlAuditRecord`, and removes it from the in-memory registry -- all inside the single
/// `write_lock`-guarded critical section shared with every other registry writer (see module
/// docs). Idempotent: a version already removed from a prior successful apply is simply not
/// recomputed as eligible, so a repeat call is a no-op, not an error.
///
/// Partial-failure judgment call: if a deletion errors partway through the eligible list
/// (e.g. a transient object-store error), versions successfully deleted BEFORE the error are
/// still persisted (removed from the registry, audited) rather than rolled back -- an S3
/// delete cannot be un-done, so recording what actually happened is safer than pretending the
/// whole call failed atomically. The response in that case is a 500 naming which version(s)
/// failed; the caller can retry the apply, which will only re-attempt the remaining eligible
/// versions.
async fn ttl_apply(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Json<TtlApplyResponse>, axum::response::Response> {
    if !is_leader(&state.leader_state) {
        return Err(not_leader().into_response_pair());
    }

    let _guard = state.write_lock.lock().await;
    let mut map = load_registry_map(&state.registry_path)
        .await
        .map_err(|e| internal_error(e).into_response_pair())?;
    let entry = map
        .get_mut(&id)
        .ok_or_else(|| table_not_found(&id).into_response_pair())?;

    let policy = entry.ttl_policy.unwrap_or_default();
    let now = chrono::Utc::now();
    let eligible_ids: Vec<String> =
        catalog_core::ttl_eligible_versions(&policy, &entry.versions, now)
            .into_iter()
            .map(|v| v.version_id.clone())
            .collect();

    let mut deleted = Vec::new();
    let mut reclaimed_bytes = 0u64;
    let mut audit_records: Vec<TtlAuditRecord> = Vec::new();
    // Version ids whose delete failed this call (raw errors are logged, not returned).
    let mut failed_versions: Vec<String> = Vec::new();

    for vid in &eligible_ids {
        let Some(pos) = entry.versions.iter().position(|v| &v.version_id == vid) else {
            continue;
        };
        let version = entry.versions[pos].clone();
        // Defense in depth: never delete a protected version even if eligibility computation
        // somehow said otherwise (it shouldn't -- `ttl_eligible_versions` already excludes
        // `protected` versions -- but this is the last line of defense before an irreversible
        // S3 delete).
        if version.protected {
            continue;
        }

        // A snapshot_path that doesn't resolve under the sweep root is a hard error, not a
        // silent success: without a valid prefix we cannot delete anything, so keep the version
        // in the registry and report it rather than removing it while its bytes remain on S3.
        // A snapshot_path that doesn't resolve under the sweep root is a hard error, not a
        // silent success: without a valid prefix we cannot delete anything, so keep the version
        // in the registry and report it rather than removing it while its bytes remain on S3.
        // The raw error is logged; only the (caller-owned) version id goes into the response.
        let prefix = match state.sweep_cfg.path_for(&version.snapshot_path) {
            Ok(prefix) => prefix,
            Err(e) => {
                metrics::record_ttl_delete(false);
                tracing::error!(table = %id, version = %vid, error = %e, "ttl delete: unresolvable snapshot path");
                failed_versions.push(vid.clone());
                continue;
            }
        };
        match catalog_store::delete_prefix(state.sweep_cfg.store.as_ref(), &prefix).await {
            Ok(()) => {
                metrics::record_ttl_delete(true);
                metrics::record_s3_op("ttl_delete", true);
                audit_records.push(TtlAuditRecord {
                    table_id: id.clone(),
                    version_id: vid.clone(),
                    deleted_at: now,
                    reclaimed_bytes: version.storage_bytes_total,
                    policy_snapshot: policy,
                    actor: "ttl-engine".to_string(),
                });
                reclaimed_bytes += version.storage_bytes_total;
                deleted.push(vid.clone());
                entry.versions.remove(pos);
            }
            Err(e) => {
                metrics::record_ttl_delete(false);
                metrics::record_s3_op("ttl_delete", false);
                tracing::error!(table = %id, version = %vid, error = %e, "ttl delete failed");
                failed_versions.push(vid.clone());
            }
        }
    }

    // The deleted versions are gone from `entry.versions`; the denormalized `aux_latest` must
    // be recomputed so it never points at a version that was just hard-deleted from S3.
    catalog_core::recompute_aux_latest(entry);

    // Durably record the deletions BEFORE persisting their removal from the registry. If the
    // process crashes between these two writes, the version is still listed in the registry
    // (self-healing: the next apply re-computes it as eligible and the delete is idempotent),
    // but it is already audited -- the audit is the only durable evidence of an irreversible
    // hard-delete, so it must never be the write that gets lost.
    if !audit_records.is_empty() {
        catalog_core::append_ttl_audit(&state.ttl_audit_path, &audit_records)
            .await
            .map_err(|e| {
                metrics::record_ttl_apply(false);
                internal_error(e).into_response_pair()
            })?;
    }

    persist_and_refresh_cache(&state, map).await.map_err(|e| {
        metrics::record_ttl_apply(false);
        internal_error(e).into_response_pair()
    })?;

    if !failed_versions.is_empty() {
        metrics::record_ttl_apply(false);
        // Name the versions that failed (caller-owned ids, safe to return) so the caller can
        // retry them; the underlying errors were logged per-version above.
        return Err(error(
            StatusCode::INTERNAL_SERVER_ERROR,
            0,
            format!("failed to delete versions: {}", failed_versions.join(", ")),
        )
        .into_response_pair());
    }

    // The reclaimable-bytes gauge is maintained by the sweep from ground truth (leader-only),
    // so apply does not poke it here — the next sweep reflects the post-delete total.
    metrics::record_ttl_apply(true);

    Ok(Json(TtlApplyResponse {
        table_id: id,
        deleted,
        reclaimed_bytes,
    }))
}

/// Map an internal failure to a 500. The full error (which may carry object-store paths,
/// backtraces, or other internal detail) is logged, not returned to the client -- the response
/// body is a fixed generic message so nothing internal leaks over the wire.
fn internal_error(e: anyhow::Error) -> (StatusCode, Json<ErrorResponse>) {
    tracing::error!(error = %e, "request failed with an internal error");
    error(
        StatusCode::INTERNAL_SERVER_ERROR,
        0,
        "internal server error",
    )
}

/// Tower/axum middleware that records HTTP RED metrics (`catalog_http_requests_total` and
/// `catalog_http_request_duration_seconds`) for every request passing through the API router.
///
/// `route` is taken from axum's `MatchedPath` (e.g. `/v1/table/:id`), not the concrete URI
/// (e.g. `/v1/table/smoke_test`), keeping label cardinality bounded regardless of table IDs.
/// `class` is derived from the route pattern via `metrics::route_class`.
async fn red_middleware(
    matched_path: Option<MatchedPath>,
    method: axum::http::Method,
    request: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let start = Instant::now();

    // Intern the matched-path pattern to a `&'static str` for use as a metric label value.
    // Each distinct route pattern is leaked at most once (via `metrics::intern_route`); repeat
    // requests with the same pattern reuse the same `&'static str`, so this does not grow the
    // heap per request. The label value is the route *pattern* (e.g. `/v1/table/:id`), not the
    // concrete URI, so label cardinality stays bounded regardless of table IDs.
    let route: &'static str = match matched_path {
        Some(mp) => crate::metrics::intern_route(mp.as_str()),
        None => "unknown",
    };

    let method_str: &'static str = match method.as_str() {
        "GET" => "GET",
        "POST" => "POST",
        "PUT" => "PUT",
        "DELETE" => "DELETE",
        "PATCH" => "PATCH",
        "HEAD" => "HEAD",
        "OPTIONS" => "OPTIONS",
        _ => "OTHER",
    };

    let response = next.run(request).await;

    let status_str: &'static str = match response.status().as_u16() {
        200 => "200",
        201 => "201",
        204 => "204",
        400 => "400",
        401 => "401",
        403 => "403",
        404 => "404",
        409 => "409",
        422 => "422",
        429 => "429",
        500 => "500",
        503 => "503",
        _ => "other",
    };

    let class = metrics::route_class(route);
    metrics::record_http_request(route, method_str, status_str, class, start.elapsed());

    response
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

/// Build the public `/v1` + `/ext/v1` router, fully wired to `state` (returns `Router<()>`,
/// ready to `.merge()` into the top-level app router). Every request is instrumented by
/// `red_middleware`, which records `catalog_http_requests_total` and
/// `catalog_http_request_duration_seconds` using the matched route pattern as the `route` label
/// (never the concrete URI path) so label cardinality stays bounded. The `/metrics` endpoint
/// lives on the separate internal server (`crate::internal_server`).
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
        .route("/ext/v1/tables/:id/ttl/dryrun", get(ttl_dryrun))
        .route("/ext/v1/tables/:id/ttl/audit", get(ttl_audit))
        .route("/ext/v1/tables/:id/ttl/apply", post(ttl_apply))
        .layer(axum::middleware::from_fn(red_middleware))
        .with_state(state)
}
