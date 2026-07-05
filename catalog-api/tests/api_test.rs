//! Integration tests for the public REST API (`catalog-api/src/api.rs`), driven straight
//! against the axum `Router` via `tower::ServiceExt::oneshot` -- no real TCP listener needed.
//!
//! The app is wired the way `main.rs` wires it: a derived registry snapshot on disk (written by
//! the sweep in production, seeded here) plus an authored overlay ([`catalog_store::MetaStore`])
//! backed by `InMemory` -- object-store conditional writes (ETag CAS) are the whole point of the
//! overlay, and `LocalFileSystem` doesn't implement them, so the overlay must be `InMemory` in
//! tests while the sweep/TTL data path stays on `LocalFileSystem`.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use catalog_api_lib::api::{api_router, ApiState};
use catalog_api_lib::catalog::Catalog;
use catalog_core::{
    AuxEntry, AuxFormat, Namespace, TableEntry, TableVersion, TtlPolicy, VersionShape,
};
use catalog_store::{MetaStore, SweepConfig, TableMeta};
use chrono::{TimeZone, Utc};
use http_body_util::BodyExt;
use object_store::local::LocalFileSystem;
use object_store::memory::InMemory;
use object_store::path::Path as ObjPath;
use tower::ServiceExt;

mod common;

fn fixture_version(id: &str) -> TableVersion {
    let ts = Utc.with_ymd_and_hms(2026, 6, 26, 12, 0, 0).unwrap();
    TableVersion {
        version_id: id.to_string(),
        timestamp: ts,
        snapshot_path: format!("s3://bucket/smoke_test/{id}"),
        shape: VersionShape::Full,
        partial: false,
        // Derived snapshot never carries protection; it comes from the overlay at merge time.
        protected: false,
        storage_bytes_total: 100,
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
        }],
        swept_at: ts,
    }
}

fn fixture_entry() -> TableEntry {
    let version = fixture_version("2026-06-26T12:00:00Z");
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

/// Build a test app: a fresh Lance registry snapshot on disk seeded with `entries` (the derived
/// state the sweep produces) plus an `InMemory` authored overlay seeded with `overlays` (the
/// owner/ttl_policy/protected the API mutates). `cache_ttl` is zero so every read revalidates
/// against storage -- read-your-writes is deterministic without depending on cache timing. The
/// `sweep_cfg` points at a throwaway, never-populated sweep root, fine for every test except TTL
/// `apply`, which uses `test_app_with_sweep_cfg` so it can seed real objects to delete.
async fn test_app(
    entries: Vec<TableEntry>,
    overlays: Vec<(&str, TableMeta)>,
) -> (Router, tempfile::TempDir, tempfile::TempDir) {
    let sweep_dir = tempfile::tempdir().unwrap();
    let sweep_cfg = common::sweep_config(sweep_dir.path());
    let (app, _ttl_audit, reg_dir) = test_app_with_sweep_cfg(entries, overlays, sweep_cfg).await;
    (app, reg_dir, sweep_dir)
}

/// Like `test_app`, but the caller supplies (and keeps alive) the `SweepConfig` wired into
/// `ApiState` -- needed by the TTL `apply` test, which seeds real objects into the same object
/// store `ttl_apply` deletes from. Returns the audit-log path so the test can read it back.
async fn test_app_with_sweep_cfg(
    entries: Vec<TableEntry>,
    overlays: Vec<(&str, TableMeta)>,
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

    let meta = MetaStore::new(Arc::new(InMemory::new()), ObjPath::from("_catalog/meta"));
    for (id, tm) in overlays {
        meta.mutate_meta(id, |m| *m = tm.clone()).await.unwrap();
    }

    let catalog = Catalog::new(registry_path, meta.clone(), Duration::ZERO);
    let state = ApiState::new(catalog, meta, sweep_cfg, ttl_audit_path.clone());
    (api_router(state), ttl_audit_path, dir)
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

#[tokio::test]
async fn list_tables_returns_identifier_strings() {
    let (app, _r, _s) = test_app(vec![fixture_entry()], vec![]).await;

    let resp = get(&app, "/v1/tables").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["tables"], serde_json::json!(["smoke_test"]));
}

#[tokio::test]
async fn describe_table_returns_full_detail_and_404s_for_unknown_id() {
    let (app, _r, _s) = test_app(vec![fixture_entry()], vec![]).await;

    let resp = get(&app, "/v1/table/smoke_test").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["id"], "smoke_test");
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
                .uri("/v1/table/smoke_test")
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
    let json = body_json(get(&app, "/v1/table/smoke_test").await).await;
    assert_eq!(json["owner"], "raymond");
}

#[tokio::test]
async fn bare_put_registers_a_table_with_no_body_and_rejects_malformed_json() {
    let (app, _r, _s) = test_app(vec![], vec![]).await;

    // A bare PUT (no body, no content-type) registers the table with no owner/policy — the
    // primary "add this table to the allowlist" action must not require a JSON envelope.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/v1/table/just_registered")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["id"], "just_registered");
    assert!(json["owner"].is_null());

    // It is now registered (an overlay-only stub) and shows in the listing.
    let json = body_json(get(&app, "/v1/tables").await).await;
    assert_eq!(json["tables"], serde_json::json!(["just_registered"]));

    // A present-but-malformed JSON body is a 400 (not silently ignored).
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/v1/table/whatever")
                .header("content-type", "application/json")
                .body(Body::from("{not json"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // An id with a character the object store would percent-encode is refused (400) rather than
    // silently registered under a mangled overlay filename that the sweep would never find. The
    // id is URL-encoded in the request path; axum decodes it before the handler sees it.
    let resp = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/v1/table/bad%23id") // -> "bad#id" after decode
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
    // No sweep has observed it yet: the stub has authored state but no derived versions.
    assert_eq!(json["versions"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn deregister_clears_authored_overlay_but_keeps_the_derived_entry() {
    // A table present on S3 (derived snapshot) with an authored owner overlaid on top.
    let overlay = (
        "smoke_test",
        TableMeta {
            owner: Some("raymond".into()),
            ..Default::default()
        },
    );
    let (app, _r, _s) = test_app(vec![fixture_entry()], vec![overlay]).await;

    assert_eq!(
        body_json(get(&app, "/v1/table/smoke_test").await).await["owner"],
        "raymond"
    );

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

    // The derived entry survives (S3 truth is unchanged); only the authored fields are cleared.
    let json = body_json(get(&app, "/v1/table/smoke_test").await).await;
    assert_eq!(json["id"], "smoke_test");
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
        "/ext/v1/tables/smoke_test/versions/2026-06-26T12:00:00Z",
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["version_id"], "2026-06-26T12:00:00Z");

    let resp = get(&app, "/ext/v1/tables/smoke_test/versions/nope").await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(body_json(resp).await["error_code"], 11);
}

#[tokio::test]
async fn protect_endpoint_writes_the_flag_to_the_overlay_and_it_merges_into_the_view() {
    let (app, _r, _s) = test_app(vec![fixture_entry()], vec![]).await;

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
    assert_eq!(body_json(resp).await["protected"], true);

    let json = body_json(
        get(
            &app,
            "/ext/v1/tables/smoke_test/versions/2026-06-26T12:00:00Z",
        )
        .await,
    )
    .await;
    assert_eq!(json["protected"], true);
}

/// Build a TTL fixture `TableVersion` whose `snapshot_path` resolves (via `sweep_cfg.uri_for`)
/// to `<table>/<ts_dir>` in `sweep_cfg`'s backing object store -- so `ttl_apply`'s
/// `path_for(snapshot_path)` round-trips back to the real prefix a test seeded objects under.
fn ttl_fixture_version(
    sweep_cfg: &SweepConfig,
    table: &str,
    ts_dir: &str,
    days_ago: i64,
    shape: VersionShape,
) -> TableVersion {
    let ts = Utc::now() - chrono::Duration::days(days_ago);
    let snapshot_path = sweep_cfg.uri_for(&ObjPath::from(format!("{table}/{ts_dir}")));
    TableVersion {
        version_id: ts_dir.to_string(),
        timestamp: ts,
        snapshot_path,
        shape,
        partial: !matches!(shape, VersionShape::Full),
        protected: false,
        storage_bytes_total: 10,
        lance_core_bytes: 10,
        sidecar_bytes: 0,
        segments_bytes: 0,
        other_aux_bytes: 0,
        row_count: None,
        num_fragments: None,
        schema_json: None,
        num_indices: None,
        lance_version: None,
        writer_version: None,
        aux: Vec::new(),
        swept_at: ts,
    }
}

/// The overlay a TTL test seeds: the authored policy plus the protected version id. `eligible_under`
/// reads the policy and protection from the fresh overlay, so both must live here, not on the
/// derived snapshot entry.
fn ttl_overlay(protected: &str) -> TableMeta {
    TableMeta {
        ttl_policy: Some(TtlPolicy {
            keep_last_n: Some(1),
            max_age_days: Some(30),
        }),
        protected: BTreeSet::from([protected.to_string()]),
        ..Default::default()
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
    let sweep_cfg = common::sweep_config(sweep_root_dir.path());

    let eligible_old = ttl_fixture_version(
        &sweep_cfg,
        "t1",
        "2020-01-01T00-00-00",
        2000,
        VersionShape::Full,
    );
    let recent = ttl_fixture_version(
        &sweep_cfg,
        "t1",
        "2026-07-01T00-00-00",
        1,
        VersionShape::Full,
    );
    let protected_old = ttl_fixture_version(
        &sweep_cfg,
        "t1",
        "2020-06-01T00-00-00",
        1900,
        VersionShape::Full,
    );
    let partial_old = ttl_fixture_version(
        &sweep_cfg,
        "t1",
        "2019-01-01T00-00-00",
        2500,
        VersionShape::LanceOnlyPartial,
    );

    let entry = TableEntry {
        id: "t1".to_string(),
        name: "t1".to_string(),
        namespace: Namespace::new(["ns"]),
        root_location: "whatever".to_string(),
        owner: None,
        ttl_policy: None,
        last_swept: None,
        versions: vec![
            eligible_old.clone(),
            recent.clone(),
            protected_old.clone(),
            partial_old.clone(),
        ],
        aux_latest: Vec::new(),
    };
    let overlay = ("t1", ttl_overlay(&protected_old.version_id));

    let (app, _ttl_audit, _dir) =
        test_app_with_sweep_cfg(vec![entry], vec![overlay], sweep_cfg).await;

    let resp = get(&app, "/ext/v1/tables/t1/ttl/dryrun").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    // Only `eligible_old` clears both thresholds; `recent` is within keep_last_n, `protected_old`
    // is exempt via the overlay, and `partial_old`'s shape fails the safety gate.
    assert_eq!(
        json["candidates"],
        serde_json::json!([eligible_old.version_id])
    );
    assert_eq!(json["reclaimable_bytes"], eligible_old.storage_bytes_total);
}

#[tokio::test]
async fn ttl_apply_deletes_objects_writes_audit_hides_version_and_is_idempotent() {
    let sweep_root_dir = tempfile::tempdir().unwrap();
    let store = LocalFileSystem::new_with_prefix(sweep_root_dir.path()).unwrap();
    let sweep_cfg = common::sweep_config(sweep_root_dir.path());

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
    );
    let recent = ttl_fixture_version(
        &sweep_cfg,
        "t1",
        "2026-07-01T00-00-00",
        1,
        VersionShape::Full,
    );
    let protected_old = ttl_fixture_version(
        &sweep_cfg,
        "t1",
        "2020-06-01T00-00-00",
        1900,
        VersionShape::Full,
    );

    let entry = TableEntry {
        id: "t1".to_string(),
        name: "t1".to_string(),
        namespace: Namespace::new(["ns"]),
        root_location: "whatever".to_string(),
        owner: None,
        ttl_policy: None,
        last_swept: None,
        versions: vec![eligible_old.clone(), recent.clone(), protected_old.clone()],
        aux_latest: Vec::new(),
    };
    let overlay = ("t1", ttl_overlay(&protected_old.version_id));

    let (app, ttl_audit_path, _dir) =
        test_app_with_sweep_cfg(vec![entry], vec![overlay], sweep_cfg.clone()).await;

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

    // The deleted version's objects are gone; the kept and protected versions' objects remain.
    let deleted_prefix = sweep_cfg.path_for(&eligible_old.snapshot_path).unwrap();
    let deleted_bytes = catalog_store::recursive_bytes(sweep_cfg.store.as_ref(), &deleted_prefix)
        .await
        .unwrap();
    assert_eq!(deleted_bytes, 0, "deleted version's objects must be gone");

    let kept_prefix = sweep_cfg.path_for(&recent.snapshot_path).unwrap();
    let kept_bytes = catalog_store::recursive_bytes(sweep_cfg.store.as_ref(), &kept_prefix)
        .await
        .unwrap();
    assert!(kept_bytes > 0, "kept version's objects must survive");

    let protected_prefix = sweep_cfg.path_for(&protected_old.snapshot_path).unwrap();
    let protected_bytes =
        catalog_store::recursive_bytes(sweep_cfg.store.as_ref(), &protected_prefix)
            .await
            .unwrap();
    assert!(
        protected_bytes > 0,
        "protected version's objects must never be deleted"
    );

    // View no longer lists the deleted version (hidden by the `deleting` tombstone until the next
    // sweep reconciles it out of the snapshot); protected/kept versions remain.
    let json = body_json(get(&app, "/v1/table/t1").await).await;
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
    let audit = catalog_core::read_ttl_audit(&ttl_audit_path)
        .await
        .unwrap()
        .expect("audit log exists after a delete");
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].table_id, "t1");
    assert_eq!(audit[0].version_id, eligible_old.version_id);
    assert_eq!(audit[0].actor, "ttl-engine");

    // Idempotent: re-applying finds nothing eligible (the `deleting` marker excludes it) -- no-op.
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
    assert_eq!(
        body_json(resp).await["deleted"],
        serde_json::json!(Vec::<String>::new())
    );

    let audit_after = catalog_core::read_ttl_audit(&ttl_audit_path)
        .await
        .unwrap()
        .expect("audit log still exists");
    assert_eq!(
        audit_after.len(),
        1,
        "re-apply must not write a duplicate audit record"
    );
}
