//! Integration tests for the public REST API (`catalog-api/src/api.rs`), driven straight
//! against the axum `Router` via `tower::ServiceExt::oneshot` -- no real TCP listener needed.
//!
//! The app is wired the way `main.rs` wires it: a derived registry snapshot on disk (written by
//! the sync in production, seeded here) plus an authored overlay ([`catalog_store::MetaStore`])
//! backed by `InMemory` -- object-store conditional writes (ETag CAS) are the whole point of the
//! overlay, and `LocalFileSystem` doesn't implement them, so the overlay must be `InMemory` in
//! tests while the sync data path stays on `LocalFileSystem`.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use catalog_api_lib::api::{api_router, ApiState};
use catalog_api_lib::catalog::Catalog;
use catalog_api_lib::catalog_config::CatalogConfig;
use catalog_core::{AuxEntry, AuxFormat, Namespace, TableEntry, TableVersion, VersionShape};
use catalog_store::{MetaStore, TableMeta};
use chrono::{TimeZone, Utc};
use http_body_util::BodyExt;
use object_store::memory::InMemory;
use object_store::path::Path as ObjPath;
use tower::ServiceExt;

mod common;

/// The deployment scope every fixture id is composed under: region `r1`, bucket `b1`, a single
/// registered namespace `ns1`. Composite table ids in this file take the form `r1:b1:ns1:<name>`.
fn test_catalog_config() -> CatalogConfig {
    let yaml = "region: r1\nbuckets:\n  - name: b1\n    namespaces: [ns1]\n";
    serde_yaml::from_str(yaml).unwrap()
}

fn fixture_version(id: &str) -> TableVersion {
    let ts = Utc.with_ymd_and_hms(2026, 6, 26, 12, 0, 0).unwrap();
    TableVersion {
        version_id: id.to_string(),
        timestamp: ts,
        snapshot_path: format!("s3://bucket/smoke_test/{id}"),
        shape: VersionShape::Full,
        partial: false,
        storage_bytes_total: 100,
        object_count: None,
        lance_core_bytes: 60,
        sidecar_bytes: 0,
        segments_bytes: 40,
        other_aux_bytes: 0,
        row_count: Some(10),
        num_fragments: Some(1),
        schema_json: None,
        num_indices: Some(0),
        lance_version: None,
        writer_version: None,
        aux: vec![AuxEntry {
            name: "segments".to_string(),
            path: format!("s3://bucket/smoke_test/{id}/segments"),
            format: AuxFormat::Parquet,
            role: "segments".to_string(),
            storage_bytes: 40,
            fingerprint: None,
            category: None,
            dataset_path: None,
            row_count: None,
            schema_json: None,
            lance_version: None,
            writer_version: None,
        }],
        synced_at: ts,
    }
}

fn fixture_entry() -> TableEntry {
    let version = fixture_version("2026-06-26T12:00:00Z");
    TableEntry {
        id: "r1:b1:ns1:smoke_test".to_string(),
        name: "smoke_test".to_string(),
        region: "r1".to_string(),
        bucket: "b1".to_string(),
        namespace: Namespace::new(["ns1"]),
        root_location: "s3://bucket/smoke_test".to_string(),
        owner: None,
        last_synced: Some(Utc.with_ymd_and_hms(2026, 6, 26, 12, 0, 0).unwrap()),
        versions: vec![version.clone()],
        aux_latest: version.aux,
    }
}

/// Build a test app with a derived snapshot on disk and in-memory registration metadata.
/// The zero cache lifetime makes read-your-writes independent of cache timing.
async fn test_app(
    entries: Vec<TableEntry>,
    overlays: Vec<(&str, TableMeta)>,
) -> (Router, tempfile::TempDir, tempfile::TempDir) {
    let sync_dir = tempfile::tempdir().unwrap();
    let sync_cfg = common::sync_config(sync_dir.path());
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
    let meta = MetaStore::new(Arc::new(InMemory::new()), ObjPath::from("_catalog/meta"));
    for (id, tm) in overlays {
        meta.mutate_meta(id, |m| *m = tm.clone()).await.unwrap();
    }
    let catalog = Catalog::new(registry_path, meta.clone(), Duration::ZERO);
    let state = ApiState {
        catalog,
        meta,
        sync_cfg,
        catalog_cfg: Arc::new(test_catalog_config()),
    };
    let app = api_router(state);
    (app, dir, sync_dir)
}

async fn body_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

async fn get(app: &Router, uri: &str) -> axum::response::Response {
    app.clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
}

async fn put_json(app: &Router, uri: &str, body: serde_json::Value) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn list_tables_returns_identifier_strings() {
    let (app, _r, _s) = test_app(vec![fixture_entry()], vec![]).await;

    let resp = get(&app, "/v1/tables").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["tables"], serde_json::json!(["r1:b1:ns1:smoke_test"]));
}

#[tokio::test]
async fn describe_table_returns_full_detail_and_404s_for_unknown_id() {
    let (app, _r, _s) = test_app(vec![fixture_entry()], vec![]).await;

    let resp = get(&app, "/v1/table/r1:b1:ns1:smoke_test").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["id"], "r1:b1:ns1:smoke_test");
    assert_eq!(json["versions"].as_array().unwrap().len(), 1);

    let resp = get(&app, "/v1/table/does_not_exist").await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let json = body_json(resp).await;
    assert_eq!(json["error_code"], 4);
}

#[tokio::test]
async fn declare_table_writes_owner_to_overlay_and_is_visible_immediately() {
    let (app, _r, _s) = test_app(vec![fixture_entry()], vec![]).await;

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/v1/table/r1:b1:ns1:smoke_test")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"owner":"raymond"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["owner"], "raymond");

    // Read-your-writes on the same instance: the handler invalidates the cache after the overlay
    // write, so the next read merges the fresh overlay.
    let json = body_json(get(&app, "/v1/table/r1:b1:ns1:smoke_test").await).await;
    assert_eq!(json["owner"], "raymond");
}

#[tokio::test]
async fn bare_put_registers_a_table_with_no_body_and_rejects_malformed_json() {
    let (app, _r, _s) = test_app(vec![], vec![]).await;

    // A bare PUT registers the table without requiring a JSON envelope.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/v1/table/r1:b1:ns1:just_registered")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["id"], "r1:b1:ns1:just_registered");
    assert!(json["owner"].is_null());

    // It is now registered (an overlay-only stub) and shows in the listing.
    let json = body_json(get(&app, "/v1/tables").await).await;
    assert_eq!(
        json["tables"],
        serde_json::json!(["r1:b1:ns1:just_registered"])
    );

    // A present-but-malformed JSON body is a 400 (not silently ignored).
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/v1/table/r1:b1:ns1:whatever")
                .header("content-type", "application/json")
                .body(Body::from("{not json"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // An id with a character the object store would percent-encode is refused (400) rather than
    // silently registered under a mangled overlay filename that the sync would never find. The
    // id is URL-encoded in the request path; axum decodes it before the handler sees it.
    let resp = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/v1/table/r1:b1:ns1:bad%23id") // -> "bad#id" after decode
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn declare_table_on_unknown_id_materializes_an_overlay_only_stub() {
    let (app, _r, _s) = test_app(vec![], vec![]).await;

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/v1/table/r1:b1:ns1:brand_new_table")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"owner":"raymond"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["id"], "r1:b1:ns1:brand_new_table");
    assert_eq!(json["owner"], "raymond");
    // No sync has observed it yet: the stub has authored state but no derived versions.
    assert_eq!(json["versions"].as_array().unwrap().len(), 0);
}

/// Declare only accepts ids that both parse as composite ids and fall under a namespace
/// registered in the deployment's `CatalogConfig` (region `r1`, bucket `b1`, namespace `ns1`).
#[tokio::test]
async fn declare_accepts_only_ids_scoped_to_a_registered_namespace() {
    let (app, _r, _s) = test_app(vec![], vec![]).await;

    // Registered scope -> 200.
    let ok = put_json(&app, "/v1/table/r1:b1:ns1:t1", serde_json::json!({})).await;
    assert_eq!(ok.status(), StatusCode::OK);

    // Wrong bucket, wrong namespace, malformed (too few segments), and a bad segment character
    // are all refused with a 400. `%2F` is `/` percent-encoded so it survives routing as part of
    // the `:id` path segment (axum decodes it before the handler sees it) instead of splitting
    // into an extra path segment that wouldn't match the route at all.
    for (label, bad) in [
        ("r1:bX:ns1:t1", "r1:bX:ns1:t1"),
        ("r1:b1:nsX:t1", "r1:b1:nsX:t1"),
        ("r2:b1:ns1:t1", "r2:b1:ns1:t1"),
        ("plainname", "plainname"),
        ("r1:b1:ns1:bad/name", "r1:b1:ns1:bad%2Fname"),
    ] {
        let resp = put_json(&app, &format!("/v1/table/{bad}"), serde_json::json!({})).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "id {label}");
    }
}

/// `list_namespaces` returns each distinct `(bucket, namespace)` pair, not bare namespace
/// segments -- the bucket is required to build the composite id `describe_namespace` expects.
#[tokio::test]
async fn list_namespaces_carries_bucket_per_entry() {
    let (app, _r, _s) = test_app(vec![fixture_entry()], vec![]).await;

    let json = body_json(get(&app, "/v1/namespaces").await).await;
    let namespaces = json["namespaces"].as_array().unwrap();
    assert_eq!(namespaces.len(), 1);
    assert_eq!(namespaces[0]["bucket"], "b1");
    assert_eq!(namespaces[0]["namespace"], serde_json::json!(["ns1"]));
}

/// `describe_namespace`'s id is now `bucket:prefix[:prefix...]` (colon-split, not dot-split):
/// the first segment is the bucket, the rest is the namespace prefix to match against entries.
#[tokio::test]
async fn describe_namespace_uses_colon_form() {
    let (app, _r, _s) = test_app(vec![fixture_entry()], vec![]).await;

    let json = body_json(get(&app, "/v1/namespaces/b1:ns1").await).await;
    assert_eq!(json["table_count"], 1);
}

#[tokio::test]
async fn deregister_clears_authored_overlay_but_keeps_the_derived_entry() {
    // A table present on S3 (derived snapshot) with an authored owner overlaid on top.
    let overlay = (
        "r1:b1:ns1:smoke_test",
        TableMeta {
            owner: Some("raymond".into()),
        },
    );
    let (app, _r, _s) = test_app(vec![fixture_entry()], vec![overlay]).await;

    assert_eq!(
        body_json(get(&app, "/v1/table/r1:b1:ns1:smoke_test").await).await["owner"],
        "raymond"
    );

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/v1/table/r1:b1:ns1:smoke_test")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // The derived entry survives (S3 truth is unchanged); only the authored fields are cleared.
    let json = body_json(get(&app, "/v1/table/r1:b1:ns1:smoke_test").await).await;
    assert_eq!(json["id"], "r1:b1:ns1:smoke_test");
    assert!(json["owner"].is_null());

    // Deregistering a table that isn't cataloged at all is a 404.
    let resp = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/v1/table/never_existed")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn ext_get_version_returns_detail_and_404s_for_unknown_version() {
    let (app, _r, _s) = test_app(vec![fixture_entry()], vec![]).await;

    let resp = get(
        &app,
        "/ext/v1/tables/r1:b1:ns1:smoke_test/versions/2026-06-26T12:00:00Z",
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["version_id"], "2026-06-26T12:00:00Z");

    let resp = get(&app, "/ext/v1/tables/r1:b1:ns1:smoke_test/versions/nope").await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(body_json(resp).await["error_code"], 11);
}

/// The aux sample endpoint: lance aux scans its dataset_path, parquet aux dirs scan via
/// DataFusion, limits clamp, and non-tabular formats are refused — all read-only.
#[tokio::test]
async fn aux_sample_endpoint_reads_lance_and_parquet_and_rejects_others() {
    use arrow_array::{Int32Array, RecordBatch, RecordBatchIterator};
    use arrow_schema::{DataType, Field, Schema as ArrowSchema};

    let data_dir = tempfile::tempdir().unwrap();
    let schema = Arc::new(ArrowSchema::new(vec![Field::new(
        "id",
        DataType::Int32,
        false,
    )]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(Int32Array::from(vec![1, 2, 3]))],
    )
    .unwrap();

    // Real lance aux dataset.
    let lance_path = data_dir
        .path()
        .join("tags.lance")
        .to_str()
        .unwrap()
        .to_string();
    let reader = RecordBatchIterator::new(vec![Ok(batch.clone())], schema.clone());
    lance::Dataset::write(reader, &lance_path, None::<lance::dataset::WriteParams>)
        .await
        .unwrap();

    // Real parquet aux dir (one part file), written via DataFusion.
    let parquet_dir = data_dir
        .path()
        .join("segments")
        .to_str()
        .unwrap()
        .to_string();
    let ctx = datafusion::execution::context::SessionContext::new();
    ctx.register_batch("t", batch).unwrap();
    ctx.table("t")
        .await
        .unwrap()
        .write_parquet(
            &format!("{parquet_dir}/part-00000.parquet"),
            datafusion::dataframe::DataFrameWriteOptions::new().with_single_file_output(true),
            None,
        )
        .await
        .unwrap();

    // A version whose aux list points at the two real targets + one unsupported format.
    let mut version = fixture_version("2026-06-26T12:00:00Z");
    version.aux = vec![
        AuxEntry {
            name: "tags".into(),
            path: lance_path.clone(),
            format: AuxFormat::Lance,
            role: "tags".into(),
            storage_bytes: 1,
            fingerprint: None,
            category: Some("nested_sidecar".into()),
            dataset_path: Some(lance_path),
            row_count: Some(3),
            schema_json: None,
            lance_version: None,
            writer_version: None,
        },
        AuxEntry {
            name: "segments".into(),
            path: parquet_dir,
            format: AuxFormat::Parquet,
            role: "segments".into(),
            storage_bytes: 1,
            fingerprint: None,
            category: Some("sidecar".into()),
            dataset_path: None,
            row_count: None,
            schema_json: None,
            lance_version: None,
            writer_version: None,
        },
        AuxEntry {
            name: "notes".into(),
            path: "/nowhere".into(),
            format: AuxFormat::Csv,
            role: "notes".into(),
            storage_bytes: 1,
            fingerprint: None,
            category: Some("sidecar".into()),
            dataset_path: None,
            row_count: None,
            schema_json: None,
            lance_version: None,
            writer_version: None,
        },
    ];
    let mut entry = fixture_entry();
    entry.versions = vec![version];
    let (app, _r, _s) = test_app(vec![entry], vec![]).await;

    let base = "/ext/v1/tables/r1:b1:ns1:smoke_test/versions/2026-06-26T12:00:00Z/aux/sample";

    // Lance aux: all 3 rows come back with a schema; limit=2 clamps.
    let json = body_json(get(&app, &format!("{base}?name=tags")).await).await;
    assert_eq!(json["rows"].as_array().unwrap().len(), 3);
    assert_eq!(json["schema"][0]["name"], "id");
    let json = body_json(get(&app, &format!("{base}?name=tags&limit=2")).await).await;
    assert_eq!(json["rows"].as_array().unwrap().len(), 2);

    // Parquet aux dir via DataFusion.
    let json = body_json(get(&app, &format!("{base}?name=segments&limit=10")).await).await;
    assert_eq!(json["rows"].as_array().unwrap().len(), 3);
    assert_eq!(json["format"], "parquet");

    // Unsupported format is a 400; unknown aux a 404.
    let resp = get(&app, &format!("{base}?name=notes")).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let resp = get(&app, &format!("{base}?name=missing")).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn retired_catalog_features_are_not_exposed() {
    let (app, _registry, _sync) = test_app(
        vec![fixture_entry()],
        vec![("r1:b1:ns1:smoke_test", TableMeta::default())],
    )
    .await;
    for (method, uri) in [
        ("GET", "/ext/v1/storage"),
        ("GET", "/ext/v1/me"),
        ("GET", "/ext/v1/users"),
        ("GET", "/ext/v1/tables/r1:b1:ns1:smoke_test/ttl/dryrun"),
        ("GET", "/ext/v1/tables/r1:b1:ns1:smoke_test/ttl/audit"),
        ("POST", "/ext/v1/tables/r1:b1:ns1:smoke_test/ttl/apply"),
        (
            "PUT",
            "/ext/v1/tables/r1:b1:ns1:smoke_test/versions/2026-06-26T12:00:00Z/protect",
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{method} {uri}");
    }
}

#[tokio::test]
async fn api_misses_stay_404_when_the_spa_is_served() {
    let (api, _registry, _sync) = test_app(vec![fixture_entry()], vec![]).await;
    let ui = tempfile::tempdir().unwrap();
    std::fs::write(ui.path().join("index.html"), "catalog app").unwrap();
    let app = api.merge(catalog_api_lib::webui::router(ui.path().to_str().unwrap()).unwrap());

    for uri in [
        "/ext/v1/storage",
        "/ext/v1/me",
        "/ext/v1/users",
        "/v1/unknown",
    ] {
        assert_eq!(
            get(&app, uri).await.status(),
            StatusCode::NOT_FOUND,
            "{uri}"
        );
    }
    assert_eq!(get(&app, "/v1/tables").await.status(), StatusCode::OK);
    assert_eq!(get(&app, "/tables/example").await.status(), StatusCode::OK);
}
