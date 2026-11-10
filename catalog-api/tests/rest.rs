//! The REST surface end to end through the router, against real Postgres.

#[path = "../../catalog-store/tests/support/mod.rs"]
mod support;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

async fn call(app: &Router, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let body = body.map_or_else(Body::empty, |b| Body::from(b.to_string()));
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(body)
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
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
    call(app, Method::POST, uri, Some(body)).await
}

async fn get(app: &Router, uri: &str) -> (StatusCode, Value) {
    call(app, Method::GET, uri, None).await
}

fn error(status: StatusCode, code: u32) -> impl Fn((StatusCode, Value)) {
    move |(s, body)| {
        assert_eq!(s, status, "{body}");
        assert_eq!(body["code"], code, "{body}");
        assert!(body["error"].is_string(), "{body}");
    }
}

#[tokio::test]
async fn rest_catalog_routes() {
    let Some(ns) = support::namespace("api_rest").await else {
        return;
    };
    let data = tempfile::tempdir().unwrap();
    let users = support::write_dataset(data.path(), "users.lance").await;
    let ui = tempfile::tempdir().unwrap();
    std::fs::write(ui.path().join("index.html"), "<html>catalog</html>").unwrap();
    let app = catalog_api_lib::app(ns, ui.path().to_str());

    assert_eq!(get(&app, "/healthz").await.0, StatusCode::OK);
    assert_eq!(get(&app, "/readyz").await.0, StatusCode::OK);
    let (status, page) = get(&app, "/some/client/route").await;
    assert_eq!(
        (status, page),
        (StatusCode::OK, json!("<html>catalog</html>"))
    );

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

    let (status, cheap) = post(&app, "/v1/table/prod%24users/describe", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        cheap,
        json!({
            "table": "users",
            "namespace": ["prod"],
            "location": users,
            "table_uri": users,
            "properties": {"owner": "alice"},
        })
    );
    let (status, detailed) = post(
        &app,
        "/v1/table/prod%24users/describe?load_detailed_metadata=true",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detailed["version"], 1);
    assert_eq!(detailed["is_only_declared"], false);
    assert_eq!(
        detailed["stats"],
        json!({"num_deleted_rows": 0, "num_fragments": 1})
    );
    assert_eq!(detailed["schema"]["fields"][0]["name"], "id");
    assert_eq!(detailed["schema"]["fields"][0]["type"]["type"], "int32");
    assert!(detailed.get("storage_options").is_none());
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
