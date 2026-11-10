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
    /// Object-storage rate used for the storage cost estimate.
    pub storage_pricing: StoragePricing,
}

/// Object-storage price surfaced by the storage route, so the UI never hard-codes a rate.
#[derive(Debug, Clone, PartialEq)]
pub struct StoragePricing {
    /// USD per decimal GB-month (`CATALOG_STORAGE_PRICE_PER_GB_MONTH`).
    pub price_per_gb_month: f64,
    /// Human-readable tier name (`CATALOG_STORAGE_PRICE_TIER`).
    pub tier: String,
}

impl Default for StoragePricing {
    /// The contracted OCI Object Storage Active-tier rate.
    fn default() -> Self {
        Self {
            price_per_gb_month: 0.0133875,
            tier: "OCI Object Storage Active (contracted)".into(),
        }
    }
}

impl StoragePricing {
    pub fn from_env() -> Result<Self> {
        let default = Self::default();
        let price_per_gb_month = match std::env::var("CATALOG_STORAGE_PRICE_PER_GB_MONTH") {
            Ok(raw) => raw
                .parse::<f64>()
                .ok()
                .filter(|p| p.is_finite() && *p >= 0.0)
                .with_context(|| {
                    format!("CATALOG_STORAGE_PRICE_PER_GB_MONTH must be a non-negative number, got {raw:?}")
                })?,
            Err(_) => default.price_per_gb_month,
        };
        Ok(Self {
            price_per_gb_month,
            tier: env_or("CATALOG_STORAGE_PRICE_TIER", &default.tier),
        })
    }
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
            storage_pricing: StoragePricing::from_env()?,
        })
    }
}

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}
