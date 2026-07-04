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
use catalog_core::{AuxEntry, AuxFormat, Namespace, TableEntry, TableVersion, VersionShape};
use chrono::{TimeZone, Utc};
use http_body_util::BodyExt;
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

/// Build a test app: a fresh Lance registry on disk seeded with `entries`, an in-memory cache
/// pre-populated with the same entries (as the refresh loop would have done), and a fixed
/// leader/non-leader state (no real k8s Lease -- mirrors how `leader_sweep_test.rs` uses a
/// plain `AtomicBool` in place of `KubeLeaseElector`).
async fn test_app(entries: Vec<TableEntry>, leader: bool) -> (Router, String, tempfile::TempDir) {
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

    let cache: RegistryCache = Arc::new(RwLock::new(entries));
    let leader_state: LeaderState = Arc::new(AtomicBool::new(leader));
    let state = ApiState::new(registry_path.clone(), cache, leader_state);
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
