//! Env-var driven configuration for `catalog-api` on Apps Platform (Cloud Run).

use std::str::FromStr;
use std::time::Duration;

use anyhow::{Context, Result};
use catalog_store::{SqlLimits, DEFAULT_CACHE_BUDGET_BYTES};

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
    /// Allowed top-level namespace names (`CATALOG_VERTICALS`, comma-separated); empty allows any.
    pub verticals: Vec<String>,
    /// Memory the Lance session may use for cached manifests, per-version results, and index
    /// metadata (`CATALOG_CACHE_MEMORY_LIMIT_MB`, 256).
    pub cache_budget_bytes: usize,
    /// Settings of the `/catalog/v1` routes.
    pub catalog: CatalogConfig,
}

/// Settings of the `/catalog/v1` routes.
#[derive(Debug, Clone, Default)]
pub struct CatalogConfig {
    /// Object-storage rate used for the storage cost estimate.
    pub storage_pricing: StoragePricing,
    pub sql: SqlConfig,
}

impl CatalogConfig {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            storage_pricing: StoragePricing::from_env()?,
            sql: SqlConfig::from_env()?,
        })
    }
}

/// Limits of the SQL route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlConfig {
    /// `CATALOG_SQL_MAX_ROWS` (1000), `CATALOG_SQL_TIMEOUT_SECS` (60), and
    /// `CATALOG_SQL_MEMORY_LIMIT_MB` (512).
    pub limits: SqlLimits,
    /// Queries running at once per instance (`CATALOG_SQL_MAX_CONCURRENT`, 2); more are throttled.
    pub max_concurrent: usize,
}

impl Default for SqlConfig {
    fn default() -> Self {
        Self {
            limits: SqlLimits {
                max_rows: 1000,
                timeout: Duration::from_secs(60),
                memory_limit_bytes: 512 << 20,
            },
            max_concurrent: 2,
        }
    }
}

impl SqlConfig {
    pub fn from_env() -> Result<Self> {
        let default = Self::default();
        let memory_limit_mb: usize = positive_env(
            "CATALOG_SQL_MEMORY_LIMIT_MB",
            default.limits.memory_limit_bytes >> 20,
        )?;
        Ok(Self {
            limits: SqlLimits {
                max_rows: positive_env("CATALOG_SQL_MAX_ROWS", default.limits.max_rows)?,
                timeout: Duration::from_secs(positive_env(
                    "CATALOG_SQL_TIMEOUT_SECS",
                    default.limits.timeout.as_secs(),
                )?),
                memory_limit_bytes: memory_limit_mb
                    .checked_mul(1 << 20)
                    .context("CATALOG_SQL_MEMORY_LIMIT_MB is too large")?,
            },
            max_concurrent: positive_env("CATALOG_SQL_MAX_CONCURRENT", default.max_concurrent)?,
        })
    }
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
        let cache_budget_mb: usize = positive_env(
            "CATALOG_CACHE_MEMORY_LIMIT_MB",
            DEFAULT_CACHE_BUDGET_BYTES >> 20,
        )?;
        Ok(Self {
            bind_addr,
            database_url: std::env::var("DATABASE_URL").context("DATABASE_URL must be set")?,
            db_schema: std::env::var("CATALOG_DB_SCHEMA")
                .ok()
                .filter(|s| !s.is_empty()),
            webui_dir: env_or("CATALOG_WEBUI_DIR", "frontend/dist"),
            verticals: parse_list(&std::env::var("CATALOG_VERTICALS").unwrap_or_default()),
            cache_budget_bytes: cache_budget_mb
                .checked_mul(1 << 20)
                .context("CATALOG_CACHE_MEMORY_LIMIT_MB is too large")?,
            catalog: CatalogConfig::from_env()?,
        })
    }
}

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

/// Comma-separated names, trimmed, with empty entries dropped.
fn parse_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

/// A positive integer from `name`, or `default` when unset.
fn positive_env<T: FromStr + PartialOrd + Default>(name: &str, default: T) -> Result<T> {
    match std::env::var(name) {
        Ok(raw) => raw
            .parse::<T>()
            .ok()
            .filter(|n| *n > T::default())
            .with_context(|| format!("{name} must be a positive integer, got {raw:?}")),
        Err(_) => Ok(default),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_list;

    #[test]
    fn parse_list_trims_and_drops_empty_entries() {
        assert!(parse_list("").is_empty());
        assert!(parse_list(" , ").is_empty());
        assert_eq!(
            parse_list(" automotive, ,mining "),
            ["automotive", "mining"]
        );
    }
}
