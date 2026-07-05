//! Serve the built single-page frontend from the same origin as the API.
//!
//! Serving the SPA and the REST API from one Cloud Run service means the browser talks to a
//! single origin, so there is no CORS to configure and the frontend's default same-origin `/v1`
//! and `/ext` calls just work. The assets are the Vite `dist/` output; the container copies them
//! in and points `CATALOG_WEBUI_DIR` at them. When the directory is absent (API-only deployments,
//! tests), no static routes are mounted.

use axum::Router;
use tower_http::services::{ServeDir, ServeFile};

/// Build a router that serves everything in `dir` as static files, falling back to `index.html`
/// for unmatched paths so client-side routes resolve. Returns `None` (no static routes) if `dir`
/// has no `index.html`.
pub fn router(dir: &str) -> Option<Router> {
    let path = std::path::Path::new(dir);
    let index = path.join("index.html");
    if !index.is_file() {
        tracing::warn!(dir, "no index.html in web UI dir; serving API only");
        return None;
    }
    // ServeDir handles hashed asset paths; the index fallback covers the app shell / deep links.
    let serve = ServeDir::new(path).fallback(ServeFile::new(index));
    Some(Router::new().fallback_service(serve))
}
