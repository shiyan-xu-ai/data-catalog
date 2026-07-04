//! Prometheus metrics instrumentation for `catalog-api`.
//!
//! ## Architecture
//!
//! Uses the `metrics` facade crate with `metrics-exporter-prometheus` as the global recorder.
//! All metric names carry the `catalog_` prefix (design.md §12.3). The `PrometheusHandle`
//! produced by `init()` is stored in `AppMetrics` and used to render the exposition text in
//! response to `GET /metrics`.
//!
//! ## Cardinality contract
//!
//! `table_id` and `version_id` are **never** label values (10 k tables → cardinality
//! explosion). All labels used here are bounded: `route`, `method`, `status`, `class`, `op`,
//! `result` — each taking a small finite set of values.
//!
//! ## Metric names shipped in v1.0.0
//!
//! **Request RED**
//! - `catalog_http_requests_total`          — counter   (route, method, status, class)
//! - `catalog_http_request_duration_seconds` — histogram (route, method, class)
//!
//! **Sweep**
//! - `catalog_sweep_tables_checked_total`   — counter   ()
//! - `catalog_sweep_cycle_duration_seconds` — histogram ()
//! - `catalog_snapshot_staleness_seconds`   — gauge     () — seconds since last completed sweep
//!
//! **Freshness / hydration**
//! - `catalog_hydration_ready`              — gauge     (0/1)
//!
//! **Controller lifecycle**
//! - `catalog_is_leader`                    — gauge     (0/1)
//!
//! **TTL**
//! - `catalog_ttl_deletes_total`            — counter   (result)
//! - `catalog_ttl_apply_total`              — counter   (result)
//! - `catalog_ttl_reclaimable_bytes`        — gauge     ()
//!
//! **Object store**
//! - `catalog_s3_operations_total`          — counter   (op, result) — best-effort: sweep
//!   LIST and TTL delete calls instrumented at call sites in catalog-api; deep object_store
//!   layer hooks deferred.
//!
//! **Runtime / process**
//! - `tokio_*`    via tokio-metrics (worker count, poll time, steal count …)
//! - `process_*`  via metrics-process (RSS, CPU, open fds)

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

use metrics::{describe_counter, describe_gauge, describe_histogram, Unit};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};

/// Intern a route pattern string to a `&'static str` for use as a metric label value.
///
/// The set of distinct matched-path patterns is small and fixed (one per registered route),
/// so each distinct pattern is leaked at most once and reused for every subsequent request
/// with the same pattern. This avoids the per-request `Box::leak` that would otherwise grow
/// the heap unboundedly over a long-running server's lifetime, without changing label
/// cardinality (the metrics recorder keys each series on label *value* content, so every
/// interned copy of a given pattern collapses onto the same time series).
pub fn intern_route(s: &str) -> &'static str {
    static INTERNED: OnceLock<std::sync::Mutex<HashMap<String, &'static str>>> = OnceLock::new();
    let map = INTERNED.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    let mut guard = map.lock().unwrap();
    if let Some(existing) = guard.get(s) {
        existing
    } else {
        // Leak one allocation per distinct route pattern. The set of patterns is bounded and
        // small (one per registered route), so this is a fixed, finite cost -- not the
        // unbounded per-request leak the previous `Box::leak` in `red_middleware` produced.
        let leaked: &'static str = Box::leak(s.to_string().into_boxed_str());
        guard.insert(s.to_string(), leaked);
        leaked
    }
}

/// Handle to the global Prometheus recorder. Call `.render()` to produce the text-format
/// exposition string for `GET /metrics`.
#[derive(Clone)]
pub struct AppMetrics {
    pub handle: PrometheusHandle,
}

/// Install the global `metrics` recorder and return the render handle.
///
/// Must be called exactly once, before any `metrics::*` calls. Panics if called more than
/// once (the global recorder is already set).
pub fn init() -> AppMetrics {
    let builder = PrometheusBuilder::new();

    // Register default histogram buckets that are suitable for HTTP latencies (seconds).
    // Matches Prometheus community best practice: .005,.01,.025,.05,.1,.25,.5,1,2.5,5,10.
    let builder = builder
        .set_buckets_for_metric(
            metrics_exporter_prometheus::Matcher::Prefix("catalog_http_request_duration".into()),
            &[
                0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
            ],
        )
        .expect("valid bucket config")
        .set_buckets_for_metric(
            metrics_exporter_prometheus::Matcher::Prefix("catalog_sweep_cycle_duration".into()),
            &[1.0, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0, 600.0, 1800.0],
        )
        .expect("valid bucket config");

    let handle = builder
        .install_recorder()
        .expect("failed to install Prometheus recorder");

    register_metric_metadata();

    AppMetrics { handle }
}

/// Register HELP and TYPE strings for every metric we publish.
fn register_metric_metadata() {
    // --- Request RED ---
    describe_counter!(
        "catalog_http_requests_total",
        Unit::Count,
        "Total HTTP requests handled, labelled by route, method, HTTP status, and class."
    );
    describe_histogram!(
        "catalog_http_request_duration_seconds",
        Unit::Seconds,
        "HTTP request latency in seconds, labelled by route, method, and class."
    );

    // --- Sweep ---
    describe_counter!(
        "catalog_sweep_tables_checked_total",
        Unit::Count,
        "Total number of tables visited across all sweep cycles."
    );
    describe_histogram!(
        "catalog_sweep_cycle_duration_seconds",
        Unit::Seconds,
        "Wall-clock duration of a complete sweep cycle."
    );
    describe_gauge!(
        "catalog_snapshot_staleness_seconds",
        Unit::Seconds,
        "Seconds elapsed since the last successful sweep write."
    );

    // --- Freshness / hydration ---
    describe_gauge!(
        "catalog_hydration_ready",
        "1 if the registry cache has been populated at least once, 0 otherwise."
    );

    // --- Controller lifecycle ---
    describe_gauge!(
        "catalog_is_leader",
        "1 if this pod currently holds the leader lease, 0 otherwise."
    );

    // --- TTL ---
    describe_counter!(
        "catalog_ttl_deletes_total",
        Unit::Count,
        "Total per-version TTL deletes attempted, labelled by result (ok|error)."
    );
    describe_counter!(
        "catalog_ttl_apply_total",
        Unit::Count,
        "Total TTL apply calls, labelled by result (ok|error)."
    );
    describe_gauge!(
        "catalog_ttl_reclaimable_bytes",
        Unit::Bytes,
        "Logical bytes reclaimable by TTL, updated after each dry-run or apply."
    );

    // --- Object store ---
    describe_counter!(
        "catalog_s3_operations_total",
        Unit::Count,
        "Best-effort count of object-store operations, labelled by op and result."
    );
}

/// Record one completed HTTP request.
///
/// `route`  — matched route pattern (e.g. `/v1/table/:id`).
/// `method` — HTTP method string (e.g. `GET`).
/// `status` — HTTP status code as a string (e.g. `200`).
/// `class`  — coarse route class: `metadata`, `ext`, `ttl`, `debug`, or `other`.
/// `duration` — elapsed wall time for the request.
pub fn record_http_request(
    route: &'static str,
    method: &'static str,
    status: &'static str,
    class: &'static str,
    duration: Duration,
) {
    metrics::counter!(
        "catalog_http_requests_total",
        "route" => route,
        "method" => method,
        "status" => status,
        "class" => class,
    )
    .increment(1);
    metrics::histogram!(
        "catalog_http_request_duration_seconds",
        "route" => route,
        "method" => method,
        "class" => class,
    )
    .record(duration.as_secs_f64());
}

/// Record one completed sweep cycle.
pub fn record_sweep_cycle(tables_checked: u64, duration: Duration) {
    metrics::counter!("catalog_sweep_tables_checked_total").increment(tables_checked);
    metrics::histogram!("catalog_sweep_cycle_duration_seconds").record(duration.as_secs_f64());
}

/// Update the snapshot staleness gauge (seconds since last successful sweep write).
pub fn set_snapshot_staleness(seconds: f64) {
    metrics::gauge!("catalog_snapshot_staleness_seconds").set(seconds);
}

/// Update the leader gauge.
pub fn set_is_leader(is_leader: bool) {
    metrics::gauge!("catalog_is_leader").set(if is_leader { 1.0 } else { 0.0 });
}

/// Update the hydration-ready gauge.
pub fn set_hydration_ready(ready: bool) {
    metrics::gauge!("catalog_hydration_ready").set(if ready { 1.0 } else { 0.0 });
}

/// Record one per-version TTL delete attempt.
pub fn record_ttl_delete(ok: bool) {
    let result = if ok { "ok" } else { "error" };
    metrics::counter!("catalog_ttl_deletes_total", "result" => result).increment(1);
}

/// Record one TTL apply call completion.
pub fn record_ttl_apply(ok: bool) {
    let result = if ok { "ok" } else { "error" };
    metrics::counter!("catalog_ttl_apply_total", "result" => result).increment(1);
}

/// Update the reclaimable-bytes gauge (logical bytes; may be set from dry-run or apply).
pub fn set_ttl_reclaimable_bytes(bytes: u64) {
    metrics::gauge!("catalog_ttl_reclaimable_bytes").set(bytes as f64);
}

/// Record one object-store operation.
pub fn record_s3_op(op: &'static str, ok: bool) {
    let result = if ok { "ok" } else { "error" };
    metrics::counter!("catalog_s3_operations_total", "op" => op, "result" => result).increment(1);
}

/// Classify a request path into a coarse metric class label.
///
/// Returns one of: `metadata` | `ext` | `ttl` | `debug` | `other`.
/// Used as the `class` label on HTTP RED metrics — keeps cardinality bounded
/// while still separating the main traffic segments the alerts care about.
pub fn route_class(path: &str) -> &'static str {
    if path.starts_with("/ext/v1/tables/") && path.contains("/ttl") {
        "ttl"
    } else if path.starts_with("/ext") {
        "ext"
    } else if path.starts_with("/v1") {
        "metadata"
    } else if path.starts_with("/debug") {
        "debug"
    } else {
        "other"
    }
}
