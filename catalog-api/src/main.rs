use axum::{routing::get, Router};

async fn healthz() -> &'static str {
    "ok"
}

fn app() -> Router {
    Router::new().route("/healthz", get(healthz))
}

#[tokio::main]
async fn main() {
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080")
        .await
        .expect("failed to bind listener");
    axum::serve(listener, app())
        .await
        .expect("server error");
}
