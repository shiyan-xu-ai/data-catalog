//! Integration tests for the public REST API (`catalog-api/src/api.rs`), driven straight
//! against the axum `Router` via `tower::ServiceExt::oneshot` -- no real TCP listener needed,
//! matching the level `axum::Router` test utilities operate at (Phase 4's
//! `leader_sweep_test.rs` drives loop code directly rather than binding a socket; this test
//! does the analogous thing for the HTTP layer).

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use catalog_api_lib::api::{api_router, ApiState};
use catalog_api_lib::leader::LeaderState;
use catalog_api_lib::registry_cache::RegistryCache;
use catalog_api_lib::registry_lock;
use catalog_core::{
    AuxEntry, AuxFormat, Namespace, TableEntry, TableVersion, TtlPolicy, VersionShape,
};
use catalog_store::SweepConfig;
use chrono::{TimeZone, Utc};
use http_body_util::BodyExt;
use object_store::local::LocalFileSystem;
use object_store::path::Path as ObjPath;
use tokio::sync::RwLock;
use tower::ServiceExt;

fn fixture_version(id: &str, protected: bool) -> TableVersion {
    let ts = Utc.with_ymd_and_hms(2026, 6, 26, 12, 0, 0).unwrap();
    TableVersion {
        version_id: id.to_string(),
        timestamp: ts,
        snapshot_path: format!("s3://bucket/smoke_test/{id}"),
        shape: VersionShape::Full,
        partial: false,
        protected,
        storage_bytes_total: 100,
        lance_core_bytes: 60,
        sidecar_bytes: 0,
        segments_bytes: 40,
        other_aux_bytes: 0,
        row_count: Some(10),
        num_fragments: Some(1),
        schema_json: None,
        num_indices: Some(0),
        aux: vec![AuxEntry {
            name: "segments".to_string(),
            path: format!("s3://bucket/smoke_test/{id}/segments"),
            format: AuxFormat::Parquet,
            role: "segments".to_string(),
            storage_bytes: 40,
            fingerprint: None,
        }],
        swept_at: ts,
    }
}

fn fixture_entry() -> TableEntry {
    let version = fixture_version("2026-06-26T12:00:00Z", false);
    TableEntry {
        id: "smoke_test".to_string(),
        name: "smoke_test".to_string(),
        namespace: Namespace::new(["scenario_dataset_export"]),
        root_location: "s3://bucket/smoke_test".to_string(),
        owner: None,
        ttl_policy: None,
        last_swept: Some(Utc.with_ymd_and_hms(2026, 6, 26, 12, 0, 0).unwrap()),
        versions: vec![version.clone()],
        aux_latest: version.aux,
    }
}

fn local_sweep_config(root: &std::path::Path) -> SweepConfig {
    let store = Arc::new(LocalFileSystem::new_with_prefix(root).unwrap());
    SweepConfig::new(store, ObjPath::from(""), root.to_str().unwrap().to_string())
}

/// Build a test app: a fresh Lance registry on disk seeded with `entries`, an in-memory cache
/// pre-populated with the same entries (as the refresh loop would have done), and a fixed
/// leader/non-leader state (no real k8s Lease -- mirrors how `leader_sweep_test.rs` uses a
/// plain `AtomicBool` in place of `KubeLeaseElector`). The `ApiState`'s `sweep_cfg` points at a
/// throwaway, never-populated sweep root -- fine for every test except TTL `apply`, which uses
/// `test_app_with_sweep_cfg` instead so it can seed real objects to delete.
async fn test_app(entries: Vec<TableEntry>, leader: bool) -> (Router, String, tempfile::TempDir) {
    let sweep_root_dir = tempfile::tempdir().unwrap();
    let sweep_cfg = local_sweep_config(sweep_root_dir.path());
    test_app_with_sweep_cfg(entries, leader, sweep_cfg).await
}

/// Like `test_app`, but the caller supplies (and keeps alive) the `SweepConfig` wired into
/// `ApiState` -- needed by the TTL `apply` tests, which must seed real objects into the same
/// object store `ttl_apply` will delete from.
async fn test_app_with_sweep_cfg(
    entries: Vec<TableEntry>,
    leader: bool,
    sweep_cfg: SweepConfig,
) -> (Router, String, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let registry_path = dir
        .path()
        .join("registry.lance")
        .to_str()
        .unwrap()
        .to_string();
    catalog_core::write_registry(&registry_path, &entries)
        .await
        .unwrap();
    let ttl_audit_path = dir
        .path()
        .join("ttl_audit.lance")
        .to_str()
        .unwrap()
        .to_string();

    let cache: RegistryCache = Arc::new(RwLock::new(entries));
    let leader_state: LeaderState = Arc::new(AtomicBool::new(leader));
    let write_lock = registry_lock::new_registry_write_lock();
    let state = ApiState::new(
        registry_path.clone(),
        cache,
        leader_state,
        write_lock,
        sweep_cfg,
        ttl_audit_path,
    );
    (api_router(state), registry_path, dir)
}

async fn body_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn list_tables_returns_identifier_strings() {
    let (app, _path, _dir) = test_app(vec![fixture_entry()], true).await;

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/v1/tables")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["tables"], serde_json::json!(["smoke_test"]));
}

#[tokio::test]
async fn describe_table_returns_full_detail_and_404s_for_unknown_id() {
    let (app, _path, _dir) = test_app(vec![fixture_entry()], true).await;

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/table/smoke_test")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["id"], "smoke_test");
    assert_eq!(json["versions"].as_array().unwrap().len(), 1);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/v1/table/does_not_exist")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let json = body_json(resp).await;
    assert_eq!(json["error_code"], 4);
}

#[tokio::test]
async fn declare_table_sets_owner_and_is_visible_on_subsequent_describe() {
    let (app, _path, _dir) = test_app(vec![fixture_entry()], true).await;

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/v1/table/smoke_test")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"owner":"raymond"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["owner"], "raymond");

    // Subsequent DescribeTable on the SAME app instance sees the change immediately -- the
    // handler refreshes the in-process cache synchronously after the write, rather than
    // waiting on the periodic refresh loop (which isn't running at all in this test).
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/v1/table/smoke_test")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let json = body_json(resp).await;
    assert_eq!(json["owner"], "raymond");
}

#[tokio::test]
async fn declare_table_on_unknown_id_creates_a_stub_entry() {
    let (app, _path, _dir) = test_app(vec![], true).await;

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/v1/table/brand_new_table")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"owner":"raymond"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["id"], "brand_new_table");
    assert_eq!(json["owner"], "raymond");
    assert_eq!(json["versions"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn mutation_on_a_non_leader_pod_is_rejected_with_503() {
    let (app, _path, _dir) = test_app(vec![fixture_entry()], false).await;

    let resp = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/v1/table/smoke_test")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"owner":"raymond"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn deregister_table_removes_it_and_subsequent_describe_404s() {
    let (app, _path, _dir) = test_app(vec![fixture_entry()], true).await;

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/v1/table/smoke_test")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/v1/table/smoke_test")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn ext_list_tables_returns_enriched_data_with_versions_and_aux() {
    let (app, _path, _dir) = test_app(vec![fixture_entry()], true).await;

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/ext/v1/tables?expand=versions,stats,aux")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let tables = json.as_array().unwrap();
    assert_eq!(tables.len(), 1);
    let versions = tables[0]["versions"].as_array().unwrap();
    assert_eq!(versions.len(), 1);
    assert!(!versions[0]["aux"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn ext_get_version_returns_single_version_detail() {
    let (app, _path, _dir) = test_app(vec![fixture_entry()], true).await;

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/ext/v1/tables/smoke_test/versions/2026-06-26T12:00:00Z")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["version_id"], "2026-06-26T12:00:00Z");
}

#[tokio::test]
async fn protect_endpoint_sets_the_flag_and_is_visible_via_version_detail() {
    let (app, _path, _dir) = test_app(vec![fixture_entry()], true).await;

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/ext/v1/tables/smoke_test/versions/2026-06-26T12:00:00Z/protect")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"protected":true}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["protected"], true);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/ext/v1/tables/smoke_test/versions/2026-06-26T12:00:00Z")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let json = body_json(resp).await;
    assert_eq!(json["protected"], true);
}

/// Build a TTL fixture `TableVersion` whose `snapshot_path` resolves (via `sweep_cfg.uri_for`)
/// to `<table>/<ts_dir>` in `sweep_cfg`'s backing object store -- so `ttl_apply`'s
/// `path_for(snapshot_path)` round-trips back to the real prefix a test may have seeded
/// objects under.
fn ttl_fixture_version(
    sweep_cfg: &SweepConfig,
    table: &str,
    ts_dir: &str,
    days_ago: i64,
    shape: VersionShape,
    protected: bool,
) -> TableVersion {
    let ts = Utc::now() - chrono::Duration::days(days_ago);
    let snapshot_path = sweep_cfg.uri_for(&ObjPath::from(format!("{table}/{ts_dir}")));
    TableVersion {
        version_id: ts_dir.to_string(),
        timestamp: ts,
        snapshot_path,
        shape,
        partial: !matches!(shape, VersionShape::Full),
        protected,
        storage_bytes_total: 10,
        lance_core_bytes: 10,
        sidecar_bytes: 0,
        segments_bytes: 0,
        other_aux_bytes: 0,
        row_count: None,
        num_fragments: None,
        schema_json: None,
        num_indices: None,
        aux: Vec::new(),
        swept_at: ts,
    }
}

async fn write_fake_object(store: &LocalFileSystem, path: &str, content: &[u8]) {
    use object_store::{ObjectStoreExt, PutPayload};
    store
        .put(&ObjPath::from(path), PutPayload::from(content.to_vec()))
        .await
        .expect("write fake object");
}

#[tokio::test]
async fn ttl_dryrun_returns_candidates_and_reclaimable_bytes_for_a_mixed_fixture() {
    let sweep_root_dir = tempfile::tempdir().unwrap();
    let sweep_cfg = local_sweep_config(sweep_root_dir.path());

    let eligible_old = ttl_fixture_version(
        &sweep_cfg,
        "t1",
        "2020-01-01T00-00-00",
        2000,
        VersionShape::Full,
        false,
    );
    let recent = ttl_fixture_version(
        &sweep_cfg,
        "t1",
        "2026-07-01T00-00-00",
        1,
        VersionShape::Full,
        false,
    );
    let protected_old = ttl_fixture_version(
        &sweep_cfg,
        "t1",
        "2020-06-01T00-00-00",
        1900,
        VersionShape::Full,
        true,
    );
    let partial_old = ttl_fixture_version(
        &sweep_cfg,
        "t1",
        "2019-01-01T00-00-00",
        2500,
        VersionShape::LanceOnlyPartial,
        false,
    );

    let entry = TableEntry {
        id: "t1".to_string(),
        name: "t1".to_string(),
        namespace: Namespace::new(["ns"]),
        root_location: "whatever".to_string(),
        owner: None,
        ttl_policy: Some(TtlPolicy {
            keep_last_n: Some(1),
            max_age_days: Some(30),
        }),
        last_swept: None,
        versions: vec![
            eligible_old.clone(),
            recent.clone(),
            protected_old.clone(),
            partial_old.clone(),
        ],
        aux_latest: Vec::new(),
    };

    let (app, _path, _dir) = test_app_with_sweep_cfg(vec![entry], true, sweep_cfg).await;

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/ext/v1/tables/t1/ttl/dryrun")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    // Only `eligible_old` clears both thresholds; `recent` is within keep_last_n, `protected_old`
    // is exempt regardless of policy, and `partial_old`'s shape fails the safety gate.
    assert_eq!(
        json["candidates"],
        serde_json::json!([eligible_old.version_id])
    );
    assert_eq!(json["reclaimable_bytes"], eligible_old.storage_bytes_total);
}

#[tokio::test]
async fn ttl_apply_deletes_objects_writes_audit_removes_version_and_is_idempotent() {
    let sweep_root_dir = tempfile::tempdir().unwrap();
    let store = LocalFileSystem::new_with_prefix(sweep_root_dir.path()).unwrap();
    let sweep_cfg = local_sweep_config(sweep_root_dir.path());

    write_fake_object(
        &store,
        "t1/2020-01-01T00-00-00/dataset.lance/_versions/1.manifest",
        b"old",
    )
    .await;
    write_fake_object(
        &store,
        "t1/2026-07-01T00-00-00/dataset.lance/_versions/1.manifest",
        b"recent",
    )
    .await;
    write_fake_object(
        &store,
        "t1/2020-06-01T00-00-00/dataset.lance/_versions/1.manifest",
        b"protected",
    )
    .await;

    let eligible_old = ttl_fixture_version(
        &sweep_cfg,
        "t1",
        "2020-01-01T00-00-00",
        2000,
        VersionShape::Full,
        false,
    );
    let recent = ttl_fixture_version(
        &sweep_cfg,
        "t1",
        "2026-07-01T00-00-00",
        1,
        VersionShape::Full,
        false,
    );
    let protected_old = ttl_fixture_version(
        &sweep_cfg,
        "t1",
        "2020-06-01T00-00-00",
        1900,
        VersionShape::Full,
        true,
    );

    let entry = TableEntry {
        id: "t1".to_string(),
        name: "t1".to_string(),
        namespace: Namespace::new(["ns"]),
        root_location: "whatever".to_string(),
        owner: None,
        ttl_policy: Some(TtlPolicy {
            keep_last_n: Some(1),
            max_age_days: Some(30),
        }),
        last_swept: None,
        versions: vec![eligible_old.clone(), recent.clone(), protected_old.clone()],
        aux_latest: Vec::new(),
    };

    let (app, _registry_path, dir) =
        test_app_with_sweep_cfg(vec![entry], true, sweep_cfg.clone()).await;
    let ttl_audit_path = dir
        .path()
        .join("ttl_audit.lance")
        .to_str()
        .unwrap()
        .to_string();

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/ext/v1/tables/t1/ttl/apply")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(
        json["deleted"],
        serde_json::json!([eligible_old.version_id])
    );

    // The deleted version's objects are gone; the kept and protected versions' objects
    // remain untouched.
    let deleted_prefix = sweep_cfg.path_for(&eligible_old.snapshot_path);
    let deleted_bytes = catalog_store::recursive_bytes(sweep_cfg.store.as_ref(), &deleted_prefix)
        .await
        .unwrap();
    assert_eq!(deleted_bytes, 0, "deleted version's objects must be gone");

    let kept_prefix = sweep_cfg.path_for(&recent.snapshot_path);
    let kept_bytes = catalog_store::recursive_bytes(sweep_cfg.store.as_ref(), &kept_prefix)
        .await
        .unwrap();
    assert!(kept_bytes > 0, "kept version's objects must survive");

    let protected_prefix = sweep_cfg.path_for(&protected_old.snapshot_path);
    let protected_bytes =
        catalog_store::recursive_bytes(sweep_cfg.store.as_ref(), &protected_prefix)
            .await
            .unwrap();
    assert!(
        protected_bytes > 0,
        "protected version's objects must never be deleted"
    );

    // Registry no longer lists the deleted version; protected/kept versions remain.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/table/t1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let json = body_json(resp).await;
    let mut version_ids: Vec<String> = json["versions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["version_id"].as_str().unwrap().to_string())
        .collect();
    version_ids.sort();
    assert_eq!(
        version_ids,
        vec![protected_old.version_id.clone(), recent.version_id.clone()]
    );

    // Audit record written for the deleted version only.
    let audit = catalog_core::read_ttl_audit(&ttl_audit_path).await.unwrap();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].table_id, "t1");
    assert_eq!(audit[0].version_id, eligible_old.version_id);
    assert_eq!(audit[0].actor, "ttl-engine");

    // Idempotent: re-applying finds nothing eligible (already deleted) -- no-op, not an error.
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/ext/v1/tables/t1/ttl/apply")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["deleted"], serde_json::json!(Vec::<String>::new()));

    let audit_after = catalog_core::read_ttl_audit(&ttl_audit_path).await.unwrap();
    assert_eq!(
        audit_after.len(),
        1,
        "re-apply must not write a duplicate audit record"
    );
}

#[tokio::test]
async fn cors_headers_are_present_on_a_response() {
    let (app, _path, _dir) = test_app(vec![fixture_entry()], true).await;

    // CORS is applied at the top-level app router in main.rs, not `api_router` itself
    // (`api_router` is merged into the app before the CorsLayer is applied) -- so wrap it here
    // the same way `main.rs::app()` does, to exercise the actual layered behavior.
    let app = app.layer(tower_http::cors::CorsLayer::permissive());

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/v1/tables")
                .header("origin", "http://example.com")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp.headers().contains_key("access-control-allow-origin"));
}
