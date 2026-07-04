//! Isolated test for the snapshot staleness gauge. This lives in its own integration test
//! binary so it gets a dedicated process-global Prometheus recorder, avoiding contention with
//! other test binaries that also call `metrics::init()`.

use catalog_api_lib::metrics;

/// Verify that `set_snapshot_staleness` can record a non-zero value and that the exposition
/// text reflects it. The staleness gauge updater in production sets this to elapsed seconds
/// since the last sweep; this test confirms the gauge is not stuck at zero.
#[tokio::test]
async fn staleness_gauge_reports_nonzero_value() {
    let app_metrics = metrics::init();

    // Set a non-zero staleness as the background updater would do between sweeps, then render
    // immediately so no interleaving can reset the gauge before we read it.
    metrics::set_snapshot_staleness(42.0);
    let body = app_metrics.handle.render();

    assert!(
        body.contains("catalog_snapshot_staleness_seconds 42"),
        "staleness gauge must report the non-zero value set by the updater; body:\n{body}"
    );
}
