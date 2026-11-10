//! Env-var driven configuration for `catalog-api` on Apps Platform (Cloud Run).

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct AppConfig {
    /// Address the HTTP server binds. Cloud Run injects `PORT`; else `CATALOG_BIND_ADDR`,
    /// default `0.0.0.0:8080`.
    pub bind_addr: String,
    /// Postgres connection URL (`DATABASE_URL`, required).
    pub database_url: String,
    /// Schema holding the catalog tables (`CATALOG_DB_SCHEMA`); created if missing and used as
    /// the connection `search_path`. Unset: the database's default `search_path`.
    pub db_schema: Option<String>,
    /// Directory holding the built frontend (`CATALOG_WEBUI_DIR`, default `frontend/dist`).
    /// Served at `/` when it contains an `index.html`.
    pub webui_dir: String,
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        let bind_addr = match std::env::var("PORT") {
            Ok(port) => format!("0.0.0.0:{port}"),
            Err(_) => env_or("CATALOG_BIND_ADDR", "0.0.0.0:8080"),
        };
        Ok(Self {
            bind_addr,
            database_url: std::env::var("DATABASE_URL").context("DATABASE_URL must be set")?,
            db_schema: std::env::var("CATALOG_DB_SCHEMA")
                .ok()
                .filter(|s| !s.is_empty()),
            webui_dir: env_or("CATALOG_WEBUI_DIR", "frontend/dist"),
        })
    }
}

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}
