//! The REST surface end to end through the router, against real Postgres.

#[path = "../../catalog-store/tests/support/mod.rs"]
mod support;

use std::time::Duration;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tokio::task::JoinHandle;
use tower::ServiceExt;

/// Sends a request as `user`, the value of IAP's user header; `None` sends none.
async fn call(
    app: &Router,
    method: Method,
    uri: &str,
    body: Option<Value>,
    user: Option<&str>,
) -> (StatusCode, Value) {
    let body = body.map_or_else(Body::empty, |b| Body::from(b.to_string()));
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(user) = user {
        request = request.header("x-goog-authenticated-user-email", user);
    }
    let response = app
        .clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
    };
    (status, value)
}

async fn post(app: &Router, uri: &str, body: Value) -> (StatusCode, Value) {
    call(app, Method::POST, uri, Some(body), None).await
}

async fn post_as(app: &Router, user: Option<&str>, uri: &str, body: Value) -> (StatusCode, Value) {
    call(app, Method::POST, uri, Some(body), user).await
}

async fn get(app: &Router, uri: &str) -> (StatusCode, Value) {
    call(app, Method::GET, uri, None, None).await
}

fn error(status: StatusCode, code: u32) -> impl Fn((StatusCode, Value)) {
    move |(s, body)| {
        assert_eq!(s, status, "{body}");
        assert_eq!(body["code"], code, "{body}");
        assert!(body["error"].is_string(), "{body}");
    }
}

fn named<'a>(categories: &'a Value, name: &str) -> &'a Value {
    categories
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name)
        .unwrap()
}

#[tokio::test]
async fn rest_catalog_routes() {
    let Some(ns) = support::namespace("api_rest").await else {
        return;
    };
    let data = tempfile::tempdir().unwrap();
    let users = support::write_dataset(data.path(), "users.lance").await;
    // Version 2 adds the `id_idx` scalar index.
    support::add_scalar_index(&users).await;
    let ui = tempfile::tempdir().unwrap();
    std::fs::write(ui.path().join("index.html"), "<html>catalog</html>").unwrap();
    let mut catalog = catalog_api_lib::config::CatalogConfig::default();
    // A low row cap makes the three-row fixture exercise truncation, and a low byte cap lets a
    // wide row exceed it.
    catalog.sql.limits.max_rows = 2;
    catalog.sql.limits.max_result_bytes = 256;
    let pricing = catalog.storage_pricing.clone();
    let app = catalog_api_lib::app(ns, catalog, ui.path().to_str()).unwrap();

    assert_eq!(get(&app, "/healthz").await.0, StatusCode::OK);
    assert_eq!(get(&app, "/readyz").await.0, StatusCode::OK);
    let (status, page) = get(&app, "/some/client/route").await;
    assert_eq!(
        (status, page),
        (StatusCode::OK, json!("<html>catalog</html>"))
    );

    // Only the default `$` delimiter is accepted.
    assert_eq!(
        get(&app, "/v1/namespace/%24/list?delimiter=%24").await.0,
        StatusCode::OK
    );
    error(StatusCode::BAD_REQUEST, 13)(get(&app, "/v1/namespace/prod/list?delimiter=.").await);

    // Namespaces: `$` path ids, URL-encoded, root is `$`.
    let (status, body) = post(
        &app,
        "/v1/namespace/prod/create",
        json!({"properties": {"team": "data"}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"properties": {"team": "data"}}));
    error(StatusCode::CONFLICT, 2)(post(&app, "/v1/namespace/prod/create", json!({})).await);
    assert_eq!(
        post(&app, "/v1/namespace/prod%24analytics/create", json!({}))
            .await
            .0,
        StatusCode::OK
    );
    error(StatusCode::NOT_FOUND, 1)(post(&app, "/v1/namespace/dev%24x/create", json!({})).await);
    assert_eq!(
        get(&app, "/v1/namespace/%24/list").await,
        (StatusCode::OK, json!({"namespaces": ["prod"]}))
    );
    assert_eq!(
        get(&app, "/v1/namespace/prod/list?limit=1&page_token=").await,
        (StatusCode::OK, json!({"namespaces": ["analytics"]}))
    );
    error(StatusCode::BAD_REQUEST, 13)(get(&app, "/v1/namespace/prod/list?limit=many").await);
    assert_eq!(
        post(&app, "/v1/namespace/prod/describe", json!({})).await,
        (StatusCode::OK, json!({"properties": {"team": "data"}}))
    );
    assert_eq!(
        post(&app, "/v1/namespace/prod%24analytics/exists", json!({})).await,
        (StatusCode::OK, Value::Null)
    );
    error(StatusCode::NOT_FOUND, 1)(post(&app, "/v1/namespace/nope/exists", json!({})).await);

    // The body id must agree with the route id; malformed JSON is invalid input.
    error(StatusCode::BAD_REQUEST, 13)(
        post(&app, "/v1/namespace/prod/describe", json!({"id": ["dev"]})).await,
    );
    assert_eq!(
        post(&app, "/v1/namespace/prod/describe", json!({"id": ["prod"]}))
            .await
            .0,
        StatusCode::OK
    );
    let request = Request::post("/v1/namespace/prod/describe")
        .body(Body::from("{not json"))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    error(StatusCode::BAD_REQUEST, 13)(post(&app, "/v1/namespace/%FF/create", json!({})).await);

    // Tables.
    let (status, body) = post(
        &app,
        "/v1/table/prod%24users/register",
        json!({"location": users, "properties": {"owner": "alice"}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({"location": users, "properties": {"owner": "alice"}})
    );
    error(StatusCode::CONFLICT, 5)(
        post(
            &app,
            "/v1/table/prod%24users/register",
            json!({"location": users}),
        )
        .await,
    );
    error(StatusCode::BAD_REQUEST, 13)(
        post(
            &app,
            "/v1/table/prod%24bad/register",
            json!({"location": "/nonexistent/x.lance"}),
        )
        .await,
    );
    assert_eq!(
        get(&app, "/v1/namespace/prod/table/list").await,
        (StatusCode::OK, json!({"tables": ["users"]}))
    );
    assert_eq!(
        get(&app, "/v1/table").await,
        (StatusCode::OK, json!({"tables": ["prod$users"]}))
    );

    // AE3 over HTTP: catalog-managed keys are rejected.
    error(StatusCode::BAD_REQUEST, 13)(
        post(
            &app,
            "/v1/table/prod%24users/register",
            json!({"location": users, "mode": "overwrite", "properties": {"catalog.updated_at": "x"}}),
        )
        .await,
    );

    let (status, mut cheap) = post(&app, "/v1/table/prod%24users/describe", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    let properties = cheap["properties"].take();
    assert_eq!(properties["owner"], "alice");
    assert!(properties["catalog.registered_at"].is_string());
    assert!(properties["catalog.updated_at"].is_string());
    assert_eq!(properties.as_object().unwrap().len(), 3, "{properties}");
    assert_eq!(
        cheap,
        json!({
            "table": "users",
            "namespace": ["prod"],
            "location": users,
            "table_uri": users,
            "properties": null,
        })
    );
    let (status, detailed) = post(
        &app,
        "/v1/table/prod%24users/describe?load_detailed_metadata=true",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detailed["version"], 2);
    assert_eq!(detailed["is_only_declared"], false);
    assert_eq!(
        detailed["stats"],
        json!({"num_deleted_rows": 0, "num_fragments": 1})
    );
    assert_eq!(detailed["schema"]["fields"][0]["name"], "id");
    assert_eq!(detailed["schema"]["fields"][0]["type"]["type"], "int32");
    assert_eq!(detailed["metadata"]["total_rows"], "3");
    assert!(detailed["metadata"]["lance.data_storage_version"].is_string());
    assert!(detailed.get("storage_options").is_none());

    // Read-only Lance metadata routes.
    let (status, page) = post(
        &app,
        "/v1/table/prod%24users/version/list",
        json!({"limit": 1}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["versions"][0]["version"], 1);
    assert_eq!(page["versions"][0]["metadata"]["total_rows"], "3");
    assert_eq!(page["page_token"], "1");
    let (_, page) = post(
        &app,
        "/v1/table/prod%24users/version/list?limit=1&page_token=1",
        json!({}),
    )
    .await;
    assert_eq!(page["versions"][0]["version"], 2);
    assert!(page.get("page_token").is_none());
    let (status, described) = post(
        &app,
        "/v1/table/prod%24users/version/describe",
        json!({"version": 1}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(described["version"]["version"], 1);
    let (status, indices) = post(&app, "/v1/table/prod%24users/index/list", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(indices["indexes"][0]["index_name"], "id_idx");
    assert_eq!(indices["indexes"][0]["columns"], json!(["id"]));
    assert_eq!(indices["indexes"][0]["num_indexed_rows"], 3);
    let (status, index_stats) =
        post(&app, "/v1/table/prod%24users/index/id_idx/stats", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(index_stats["num_indexed_rows"], 3);
    let (status, stats) = post(&app, "/v1/table/prod%24users/stats", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(stats["num_rows"], 3);
    assert_eq!(stats["num_indices"], 1);
    assert_eq!(stats["fragment_stats"]["num_fragments"], 1);
    assert_eq!(stats["fragment_stats"]["lengths"]["p50"], 3);
    assert_eq!(
        post(&app, "/v1/table/prod%24users/tags/list", json!({})).await,
        (StatusCode::OK, json!({"tags": {}}))
    );
    assert_eq!(
        post(
            &app,
            "/v1/table/prod%24users/count_rows",
            json!({"predicate": "id > 1"})
        )
        .await,
        (StatusCode::OK, json!(2))
    );
    let (status, missing_tag) = post(
        &app,
        "/v1/table/prod%24users/tags/version",
        json!({"tag": "release"}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{missing_tag}");
    assert_eq!(
        post(&app, "/v1/table/prod%24users/branches/list", json!({})).await,
        (StatusCode::OK, json!({"branches": {}}))
    );
    let (status, storage) = get(&app, "/catalog/v1/table/prod%24users/column-storage").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(storage["version"], 2);
    assert_eq!(storage["columns"][0]["name"], "id");
    assert_eq!(storage["columns"][1]["name"], "name");
    assert_eq!(
        storage["columns"][0]["bytes_on_disk"].as_i64().unwrap()
            + storage["columns"][1]["bytes_on_disk"].as_i64().unwrap(),
        stats["total_bytes"].as_i64().unwrap()
    );

    // Timeline pages newest first; the token is the last version returned.
    let (status, page1) = get(&app, "/catalog/v1/table/prod%24users/timeline?limit=1").await;
    assert_eq!(status, StatusCode::OK, "{page1}");
    assert_eq!(page1["events"][0]["version"], 2);
    assert_eq!(page1["events"][0]["operation"], "CreateIndex");
    assert_eq!(page1["events"][0]["details"]["added"][0]["name"], "id_idx");
    assert_eq!(page1["page_token"], "2");
    let (status, page2) = get(
        &app,
        "/catalog/v1/table/prod%24users/timeline?limit=1&page_token=2",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page2["events"][0]["operation"], "Overwrite");
    assert!(page2.get("page_token").is_none());

    let (status, measures) = get(&app, "/catalog/v1/table/prod%24users/fragment-measures").await;
    assert_eq!(status, StatusCode::OK, "{measures}");
    assert_eq!(
        (
            &measures["fragments"],
            &measures["data_files"]["max"],
            &measures["deleted_rows"]["max"]
        ),
        (&json!(1), &json!(1), &json!(0))
    );
    assert!(measures["data_bytes"]["min"].as_u64().unwrap() > 0);

    // On-storage footprint: categories partition the listing, priced per decimal GB-month.
    let (status, footprint) = get(&app, "/catalog/v1/table/prod%24users/storage").await;
    assert_eq!(status, StatusCode::OK, "{footprint}");
    assert_eq!(footprint["version"], 2);
    let on_storage = &footprint["on_storage"];
    let categories = on_storage["categories"].as_array().unwrap();
    let names: Vec<_> = categories
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "data",
            "blob",
            "index",
            "deletion",
            "manifest",
            "transaction",
            "other"
        ]
    );
    let sum = |key: &str| {
        categories
            .iter()
            .map(|c| c[key].as_u64().unwrap())
            .sum::<u64>()
    };
    let total_bytes = on_storage["total_bytes"].as_u64().unwrap();
    assert_eq!(sum("bytes"), total_bytes);
    assert_eq!(
        sum("objects"),
        on_storage["total_objects"].as_u64().unwrap()
    );
    let all = &on_storage["categories"];
    assert_eq!(
        named(all, "data")["bytes"],
        footprint["current"]["data_bytes"]
    );
    assert_eq!(
        named(all, "index")["bytes"],
        footprint["current"]["index_bytes"]
    );
    let current = &footprint["current"]["categories"];
    assert_eq!(named(current, "data"), named(all, "data"));
    assert_eq!(named(current, "index"), named(all, "index"));
    assert_eq!(named(current, "manifest")["objects"], 1);
    assert!(named(all, "manifest")["objects"].as_u64().unwrap() > 1);
    assert!(footprint["current"]["total_bytes"].as_u64().unwrap() < total_bytes);
    let price = &footprint["pricing"];
    assert_eq!(price["price_per_gb_month"], pricing.price_per_gb_month);
    assert_eq!(price["tier"], pricing.tier);
    assert_eq!(price["currency"], "USD");
    let expected = total_bytes as f64 / 1e9 * pricing.price_per_gb_month;
    assert!((price["estimated_monthly_cost"].as_f64().unwrap() - expected).abs() < 1e-12);

    // Read-only SQL over tables named by full path: typed columns, Arrow JSON values, truncation.
    let sql = |query: &str| json!({"sql": query});
    let typed_query =
        r#"SELECT id, name, id * 2 AS doubled, NULL AS nothing FROM "prod"."users" WHERE id = 1"#;
    let (status, mut typed) = post(&app, "/catalog/v1/sql", sql(typed_query)).await;
    assert_eq!(status, StatusCode::OK, "{typed}");
    assert!(typed["elapsed_ms"].take().is_u64());
    assert_eq!(
        typed,
        json!({
            "tables": [{"id": ["prod", "users"], "version": 2}],
            "columns": [
                {"name": "id", "type": "Int32"},
                {"name": "name", "type": "Utf8"},
                {"name": "doubled", "type": "Int64"},
                {"name": "nothing", "type": "Null"},
            ],
            "rows": [[1, "a", 2, null]],
            "row_count": 1,
            "truncated": false,
            "truncated_by": null,
            "elapsed_ms": null,
        })
    );
    let (status, all) = post(
        &app,
        "/catalog/v1/sql",
        sql(r#"SELECT id FROM "prod"."users" ORDER BY id"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{all}");
    assert_eq!(all["rows"], json!([[1], [2]]));
    assert_eq!(
        (&all["row_count"], &all["truncated"], &all["truncated_by"]),
        (&json!(2), &json!(true), &json!("rows"))
    );
    // Each row encodes as 204 bytes, so only one fits the 256-byte cap.
    let (status, wide) = post(
        &app,
        "/catalog/v1/sql",
        sql(r#"SELECT repeat('x', 200) FROM "prod"."users""#),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{wide}");
    assert_eq!(
        (
            &wide["row_count"],
            &wide["truncated"],
            &wide["truncated_by"]
        ),
        (&json!(1), &json!(true), &json!("bytes"))
    );
    let copied = data.path().join("copied.csv");
    for statement in [
        "CREATE TABLE t AS SELECT 1".to_string(),
        r#"INSERT INTO "prod"."users" VALUES (4, 'd')"#.to_string(),
        format!(r#"COPY "prod"."users" TO '{}'"#, copied.display()),
        r#"DROP TABLE "prod"."users""#.to_string(),
        String::new(),
    ] {
        error(StatusCode::BAD_REQUEST, 13)(post(&app, "/catalog/v1/sql", sql(&statement)).await);
    }
    assert!(!copied.exists());

    support::append_row(&users).await;
    let count = |query: &str| {
        let body = sql(query);
        let app = &app;
        async move {
            let (status, body) = post(app, "/catalog/v1/sql", body).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            (body["tables"].clone(), body["rows"].clone())
        }
    };
    let users_at = |version: u64| json!([{"id": ["prod", "users"], "version": version}]);
    assert_eq!(
        count(r#"SELECT count(*) FROM "prod"."users@v1""#).await,
        (users_at(1), json!([[3]]))
    );
    assert_eq!(
        count(r#"SELECT count(*) FROM "prod"."users""#).await,
        (users_at(2), json!([[3]]))
    );
    assert_eq!(
        count(r#"SELECT count(*) FROM "prod"."users@v3""#).await,
        (users_at(3), json!([[4]]))
    );
    assert_eq!(
        post(&app, "/catalog/v1/table/prod%24users/refresh", json!({})).await,
        (
            StatusCode::OK,
            json!({"previous_version": 2, "current_version": 3})
        )
    );
    error(StatusCode::NOT_FOUND, 4)(
        post(&app, "/catalog/v1/table/prod%24nope/refresh", json!({})).await,
    );
    assert_eq!(
        count(r#"SELECT count(*) FROM "prod"."users""#).await,
        (users_at(3), json!([[4]]))
    );
    assert_eq!(
        count(
            r#"SELECT (SELECT count(*) FROM "prod"."users@v1") AS old, count(*) AS new FROM "prod"."users""#
        )
        .await,
        (
            json!([
                {"id": ["prod", "users"], "version": 1},
                {"id": ["prod", "users"], "version": 3},
            ]),
            json!([[3, 4]])
        )
    );
    // Tables list in order of first appearance, not sorted.
    let orders = support::write_dataset(data.path(), "orders.lance").await;
    let (status, _) = post(
        &app,
        "/v1/table/prod%24orders/register",
        json!({"location": orders}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, joined) = post(
        &app,
        "/catalog/v1/sql",
        sql(r#"SELECT count(*) AS n FROM "prod"."users" u JOIN "prod"."orders" o ON o.id = u.id"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{joined}");
    assert_eq!(joined["rows"], json!([[3]]));
    assert_eq!(
        joined["tables"],
        json!([
            {"id": ["prod", "users"], "version": 3},
            {"id": ["prod", "orders"], "version": 1},
        ])
    );
    assert_eq!(
        post(&app, "/v1/table/prod%24orders/deregister", json!({}))
            .await
            .0,
        StatusCode::OK
    );
    // Each failure names the path as written.
    for (query, status, code, path) in [
        (
            r#"SELECT 1 FROM "prod"."users@v9""#,
            StatusCode::NOT_FOUND,
            11,
            r#""prod"."users@v9""#,
        ),
        (
            r#"SELECT 1 FROM "prod"."nope""#,
            StatusCode::NOT_FOUND,
            4,
            r#""prod"."nope""#,
        ),
    ] {
        let (actual, body) = post(&app, "/catalog/v1/sql", sql(query)).await;
        assert_eq!(
            (actual, body["code"].clone()),
            (status, json!(code)),
            "{body}"
        );
        assert!(body["error"].as_str().unwrap().contains(path), "{body}");
    }
    error(StatusCode::BAD_REQUEST, 13)(
        post(&app, "/catalog/v1/sql", sql("SELECT * FROM nope")).await,
    );
    assert_eq!(
        post(&app, "/v1/table/prod%24users/exists", json!({})).await,
        (StatusCode::OK, Value::Null)
    );
    error(StatusCode::NOT_FOUND, 4)(post(&app, "/v1/table/prod%24nope/describe", json!({})).await);

    // A registered table whose dataset vanished describes as not found.
    let gone = support::write_dataset(data.path(), "gone.lance").await;
    let (status, _) = post(
        &app,
        "/v1/table/prod%24gone/register",
        json!({"location": gone}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    std::fs::remove_dir_all(&gone).unwrap();
    error(StatusCode::NOT_FOUND, 4)(
        post(
            &app,
            "/v1/table/prod%24gone/describe?load_detailed_metadata=true",
            json!({}),
        )
        .await,
    );
    error(StatusCode::NOT_FOUND, 4)(
        post(&app, "/v1/table/prod%24gone/version/list", json!({})).await,
    );
    error(StatusCode::NOT_FOUND, 4)(
        get(&app, "/catalog/v1/table/prod%24gone/column-storage").await,
    );
    error(StatusCode::NOT_FOUND, 4)(get(&app, "/catalog/v1/table/prod%24gone/storage").await);
    error(StatusCode::NOT_FOUND, 4)(
        get(&app, "/catalog/v1/table/prod%24gone/fragment-measures").await,
    );
    error(StatusCode::NOT_FOUND, 4)(get(&app, "/catalog/v1/table/prod%24gone/timeline").await);
    let (status, body) = post(
        &app,
        "/catalog/v1/sql",
        sql(r#"SELECT 1 FROM "prod"."gone""#),
    )
    .await;
    assert_eq!(
        (status, body["code"].clone()),
        (StatusCode::NOT_FOUND, json!(4))
    );
    assert!(
        body["error"].as_str().unwrap().contains(r#""prod"."gone""#),
        "{body}"
    );
    assert_eq!(
        post(&app, "/v1/table/prod%24gone/deregister", json!({}))
            .await
            .0,
        StatusCode::OK
    );

    // AE4 over HTTP, then AE6: everything outside the supported set is Unsupported.
    error(StatusCode::CONFLICT, 3)(post(&app, "/v1/namespace/prod/drop", json!({})).await);
    for uri in [
        "/v1/table/prod%24users/drop",
        "/v1/table/prod%24users/query",
        "/v1/transaction/x/describe",
        "/v1/table/prod%24users/tags/create",
        "/v1/table/prod%24users/branches/create",
        "/v1/table/prod%24users/version/create",
        "/v1/table/prod%24users/create_index",
    ] {
        error(StatusCode::NOT_ACCEPTABLE, 0)(post(&app, uri, json!({})).await);
    }

    let (status, body) = post(&app, "/v1/table/prod%24users/deregister", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({"id": ["prod", "users"], "location": users, "properties": {"owner": "alice"}})
    );
    error(StatusCode::NOT_FOUND, 4)(post(&app, "/v1/table/prod%24users/exists", json!({})).await);
    assert_eq!(
        post(&app, "/v1/namespace/prod%24analytics/drop", json!({}))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        post(&app, "/v1/namespace/prod/drop", json!({})).await.0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn media_presign_signs_s3_uris() {
    let Some(pool) = support::pool("api_media").await else {
        return;
    };
    let options = [
        ("aws_access_key_id", "AKIDEXAMPLE"),
        ("aws_secret_access_key", "secret"),
        ("aws_region", "us-east-1"),
        ("aws_endpoint", "https://objects.example.com"),
    ]
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .into();
    let ns = std::sync::Arc::new(catalog_store::PostgresNamespace::new(pool, options));
    let app = catalog_api_lib::app(ns, Default::default(), None).unwrap();
    let presign = |uri: &str| {
        let path = format!("/catalog/v1/media/presign?uri={uri}");
        let app = app.clone();
        async move { get(&app, &path).await }
    };

    let (status, body) = presign("s3://media/clips/pack_00000.h265").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let url = body["url"].as_str().unwrap();
    assert!(
        url.starts_with("https://objects.example.com/media/clips/pack_00000.h265?"),
        "{url}"
    );
    assert!(
        url.contains("X-Amz-Expires=3600") && url.contains("X-Amz-Signature="),
        "{url}"
    );
    assert_eq!(body["expires_in_secs"], 3600);
    let download = body["download_url"].as_str().unwrap();
    assert!(
        download.contains(
            "response-content-disposition=attachment%3B%20filename%3D%22pack_00000.h265%22"
        ) && download.contains("X-Amz-Signature="),
        "{download}"
    );

    error(StatusCode::BAD_REQUEST, 13)(presign("https://host/a.h265").await);
    error(StatusCode::BAD_REQUEST, 13)(presign("s3://media/clips/../other/a.h265").await);
}

/// Runs until cancelled: about a trillion rows.
const ENDLESS: &str = "SELECT sum(value % 7) FROM range(1000000000000)";

fn start_endless(
    app: &Router,
    user: Option<&str>,
    query_id: &str,
) -> JoinHandle<(StatusCode, Value)> {
    let app = app.clone();
    let user = user.map(str::to_string);
    let body = json!({"sql": ENDLESS, "query_id": query_id});
    tokio::spawn(async move { post_as(&app, user.as_deref(), "/catalog/v1/sql", body).await })
}

/// Lets spawned requests reach admission, which they do before their first wait.
async fn settle() {
    tokio::time::sleep(Duration::from_millis(100)).await;
}

fn throttled((status, body): (StatusCode, Value), message: &str) {
    assert_eq!(
        (status, &body["code"]),
        (StatusCode::TOO_MANY_REQUESTS, &json!(21)),
        "{body}"
    );
    assert!(body["error"].as_str().unwrap().contains(message), "{body}");
}

async fn cancelled(request: JoinHandle<(StatusCode, Value)>) {
    let (status, body) = tokio::time::timeout(Duration::from_secs(10), request)
        .await
        .expect("the cancelled query kept running")
        .unwrap();
    assert_eq!(
        (status, &body["code"]),
        (StatusCode::BAD_REQUEST, &json!(13)),
        "{body}"
    );
    assert_eq!(body["error"], "query cancelled");
}

#[tokio::test]
async fn sql_admission_and_cancel() {
    let Some(ns) = support::namespace("api_sql_admission").await else {
        return;
    };
    let mut catalog = catalog_api_lib::config::CatalogConfig::default();
    catalog.sql.max_concurrent = 3;
    catalog.sql.max_per_user = 2;
    catalog.sql.threads = 3;
    let app = catalog_api_lib::app(ns, catalog, None).unwrap();
    let ada = Some("accounts.google.com:ada@example.com");
    let bob = Some("accounts.google.com:bob@example.com");
    let cy = Some("accounts.google.com:cy@example.com");
    let quick = || json!({"sql": "SELECT 1"});
    let cancel = |user: Option<&'static str>, query_id: &str| {
        let app = app.clone();
        let uri = format!("/catalog/v1/sql/{query_id}/cancel");
        async move { post_as(&app, user, &uri, json!({})).await }
    };

    // A user's third query is rejected while another user's still starts.
    let ada_1 = start_endless(&app, ada, "ada-1");
    let ada_2 = start_endless(&app, ada, "ada-2");
    settle().await;
    throttled(
        post_as(&app, ada, "/catalog/v1/sql", quick()).await,
        "you already have 2 queries running; wait for one or cancel it",
    );
    let bob_1 = start_endless(&app, bob, "bob-1");
    settle().await;
    // Every global slot is held.
    throttled(
        post_as(&app, cy, "/catalog/v1/sql", quick()).await,
        "the server is running its maximum of 3 queries; try again shortly",
    );

    // Only the owner cancels a query; it stops at once and leaves the registry.
    error(StatusCode::NOT_FOUND, 10)(cancel(bob, "ada-1").await);
    assert_eq!(cancel(ada, "ada-1").await.0, StatusCode::NO_CONTENT);
    cancelled(ada_1).await;
    error(StatusCode::NOT_FOUND, 10)(cancel(ada, "ada-1").await);
    // The freed slot admits the owner's next query.
    let (status, body) = post_as(&app, ada, "/catalog/v1/sql", quick()).await;
    assert_eq!(
        (status, &body["rows"]),
        (StatusCode::OK, &json!([[1]])),
        "{body}"
    );

    for (user, query_id, request) in [(ada, "ada-2", ada_2), (bob, "bob-1", bob_1)] {
        assert_eq!(cancel(user, query_id).await.0, StatusCode::NO_CONTENT);
        cancelled(request).await;
    }

    // Requests without an IAP identity share one anonymous user.
    let anonymous_1 = start_endless(&app, None, "anonymous-1");
    let anonymous_2 = start_endless(&app, None, "anonymous-2");
    settle().await;
    throttled(
        post_as(&app, None, "/catalog/v1/sql", quick()).await,
        "you already have 2 queries running",
    );
    for (query_id, request) in [("anonymous-1", anonymous_1), ("anonymous-2", anonymous_2)] {
        assert_eq!(cancel(None, query_id).await.0, StatusCode::NO_CONTENT);
        cancelled(request).await;
    }
}
