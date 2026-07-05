//! Integration tests for the internal metrics server (`GET /metrics`) and the HTTP RED
//! middleware wired on the public API router.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use catalog_api_lib::api::{api_router, ApiState};
use catalog_api_lib::internal_server::internal_router;
use catalog_api_lib::leader::LeaderState;
use catalog_api_lib::metrics;
use catalog_api_lib::registry_cache::RegistryCache;
use catalog_api_lib::registry_lock;
use catalog_core::{Namespace, TableEntry};
use http_body_util::BodyExt;
use tokio::sync::RwLock;
use tower::ServiceExt;

mod common;

/// Install the global recorder once per test binary (panics if called more than once per
/// process, so this test module owns the recorder).
fn init_metrics_once() -> catalog_api_lib::metrics::AppMetrics {
    // metrics::init() panics if the global recorder is already set. Use `std::sync::OnceLock`
    // so re-running the test binary multiple times in the same process is safe in case the
    // test framework reuses the process.
    use std::sync::OnceLock;
    static METRICS: OnceLock<catalog_api_lib::metrics::AppMetrics> = OnceLock::new();
    METRICS.get_or_init(metrics::init).clone()
}

#[tokio::test]
async fn get_metrics_returns_prometheus_exposition_text() {
    let app_metrics = init_metrics_once();

    // Record a handful of observations so the metrics actually appear in output.
    metrics::set_is_leader(true);
    metrics::set_hydration_ready(true);
    metrics::record_http_request(
        "/v1/table/:id",
        "GET",
        "200",
        "metadata",
        std::time::Duration::from_millis(12),
    );
    metrics::record_sweep_cycle(7, std::time::Duration::from_secs(3));
    metrics::set_snapshot_staleness(0.0);
    metrics::record_ttl_delete(true);
    metrics::record_ttl_apply(true);
    metrics::set_ttl_reclaimable_bytes(1024);
    metrics::record_s3_op("sweep_list", true);

    let router = internal_router(app_metrics);
    let response = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        content_type.contains("text/plain"),
        "content-type must be text/plain, got: {content_type}"
    );

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = std::str::from_utf8(&body_bytes).expect("metrics body must be UTF-8");

    // Every metric name defined in Phase 7 must appear in the exposition output.
    let required = [
        "catalog_http_requests_total",
        "catalog_http_request_duration_seconds",
        "catalog_sweep_tables_checked_total",
        "catalog_sweep_cycle_duration_seconds",
        "catalog_snapshot_staleness_seconds",
        "catalog_hydration_ready",
        "catalog_is_leader",
        "catalog_ttl_deletes_total",
        "catalog_ttl_apply_total",
        "catalog_ttl_reclaimable_bytes",
        "catalog_s3_operations_total",
    ];
    for name in required {
        assert!(
            body.contains(name),
            "exposition text missing metric: {name}\n\nbody:\n{body}"
        );
    }

    // Cardinality contract: no table_id or version_id label in any line.
    for line in body.lines() {
        assert!(
            !line.contains("table_id="),
            "table_id must never appear as a metric label; found in line: {line}"
        );
        assert!(
            !line.contains("version_id="),
            "version_id must never appear as a metric label; found in line: {line}"
        );
    }
}

#[tokio::test]
async fn get_healthz_returns_ok() {
    let app_metrics = init_metrics_once();
    let router = internal_router(app_metrics);
    let response = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body_bytes[..], b"ok");
}

// ---------------------------------------------------------------------------
// RED middleware tests
// ---------------------------------------------------------------------------

/// Build a minimal API router seeded with one table entry, wired as leader.
async fn test_api_router() -> (axum::Router, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let registry_path = dir
        .path()
        .join("registry.lance")
        .to_str()
        .unwrap()
        .to_string();
    let entry = TableEntry {
        id: "smoke_test".to_string(),
        name: "smoke_test".to_string(),
        namespace: Namespace::new(["scenario_dataset_export"]),
        root_location: "s3://bucket/smoke_test".to_string(),
        owner: None,
        ttl_policy: None,
        last_swept: None,
        versions: Vec::new(),
        aux_latest: Vec::new(),
    };
    catalog_core::write_registry(&registry_path, std::slice::from_ref(&entry))
        .await
        .unwrap();
    let cache: RegistryCache = Arc::new(RwLock::new(vec![entry]));
    let leader: LeaderState = Arc::new(AtomicBool::new(true));
    let write_lock = registry_lock::new_registry_write_lock();
    let sweep_root = tempfile::tempdir().unwrap();
    let sweep_cfg = common::sweep_config(sweep_root.path());
    let ttl_audit_path = dir
        .path()
        .join("ttl_audit.lance")
        .to_str()
        .unwrap()
        .to_string();
    let state = ApiState::new(
        registry_path,
        cache,
        leader,
        write_lock,
        sweep_cfg,
        ttl_audit_path,
    );
    (api_router(state), dir)
}

/// Render `/metrics` from the test recorder and return the exposition text.
async fn render_metrics() -> String {
    let app_metrics = init_metrics_once();
    let router = internal_router(app_metrics);
    let response = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    std::str::from_utf8(&bytes).unwrap().to_string()
}

/// Drive `GET /v1/tables` through the real API router (with RED middleware) and assert that
/// `class="metadata"` appears in the rendered metrics output. This verifies the middleware
/// correctly classifies `/v1/*` routes as the `metadata` class.
#[tokio::test]
async fn red_middleware_emits_metadata_class_for_v1_routes() {
    let _ = init_metrics_once();
    let (router, _dir) = test_api_router().await;

    let response = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/tables")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = render_metrics().await;
    assert!(
        body.contains(r#"class="metadata""#),
        "exposition text must contain class=\"metadata\" after a /v1 request; body:\n{body}"
    );
}

/// Drive `GET /v1/table/smoke_test` (a concrete path with a real table ID) through the real
/// router and verify the `route` label in the exposition text is the matched pattern
/// `/v1/table/:id`, NOT the filled-in concrete path `/v1/table/smoke_test`. This confirms
/// the middleware uses `MatchedPath` and keeps label cardinality bounded.
#[tokio::test]
async fn red_middleware_uses_route_pattern_not_concrete_path() {
    let _ = init_metrics_once();
    let (router, _dir) = test_api_router().await;

    let response = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/table/smoke_test")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = render_metrics().await;
    assert!(
        body.contains(r#"route="/v1/table/:id""#),
        "route label must be the pattern /v1/table/:id, not the concrete path; body:\n{body}"
    );
    assert!(
        !body.contains(r#"route="/v1/table/smoke_test""#),
        "concrete path /v1/table/smoke_test must never appear as a route label; body:\n{body}"
    );
}
