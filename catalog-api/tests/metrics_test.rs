//! Integration test for the internal metrics server (`GET /metrics`).
//!
//! Verifies that the Prometheus exposition text includes all metric names introduced in
//! Phase 7 (Part A). Uses the internal router directly — no real TCP listener needed.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use catalog_api_lib::internal_server::internal_router;
use catalog_api_lib::metrics;
use http_body_util::BodyExt;
use tower::ServiceExt;

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
