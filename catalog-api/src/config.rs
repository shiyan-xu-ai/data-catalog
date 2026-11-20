//! Env-var driven configuration for `catalog-api` on Apps Platform (Cloud Run).

use std::str::FromStr;
use std::time::Duration;

use anyhow::{Context, Result};
use catalog_store::{SqlLimits, DEFAULT_CACHE_BUDGET_BYTES, DEFAULT_SQL_CACHE_BYTES};

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
    /// URI prefixes table locations must fall under (`CATALOG_TABLE_LOCATIONS`,
    /// comma-separated); empty allows any.
    pub table_locations: Vec<String>,
    /// URI prefixes presigned media objects must fall under (`CATALOG_MEDIA_LOCATIONS`,
    /// comma-separated); empty allows any.
    pub media_locations: Vec<String>,
    pub cache_budget_bytes: usize,
    /// Settings of the `/catalog/v1` routes.
    pub catalog: CatalogConfig,
}

/// Settings of the `/catalog/v1` routes.
#[derive(Debug, Clone)]
pub struct CatalogConfig {
    /// Object-storage rate used for the storage cost estimate.
    pub storage_pricing: StoragePricing,
    pub sql: SqlConfig,
    /// Lifetime of presigned media URLs (`CATALOG_MEDIA_URL_TTL_MINUTES`, 60).
    pub media_url_ttl: Duration,
}

impl Default for CatalogConfig {
    fn default() -> Self {
        Self {
            storage_pricing: StoragePricing::default(),
            sql: SqlConfig::default(),
            media_url_ttl: Duration::from_secs(60 * 60),
        }
    }
}

impl CatalogConfig {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            storage_pricing: StoragePricing::from_env()?,
            sql: SqlConfig::from_env()?,
            media_url_ttl: Duration::from_secs(
                60 * positive_env("CATALOG_MEDIA_URL_TTL_MINUTES", 60u64)?,
            ),
        })
    }
}

/// Limits of the SQL route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlConfig {
    /// Per query: `CATALOG_SQL_MAX_ROWS` (1000), `CATALOG_SQL_MAX_RESULT_MB` (8), and
    /// `CATALOG_SQL_TIMEOUT_SECS` (300). The memory pool is `CATALOG_SQL_MEMORY_BUDGET_MB`
    /// (1024), the budget of all queries together, divided by `max_concurrent`.
    pub limits: SqlLimits,
    /// Queries running at once per instance (`CATALOG_SQL_MAX_CONCURRENT`, 2); more are throttled.
    pub max_concurrent: usize,
    /// Queries one user runs at once (`CATALOG_SQL_MAX_PER_USER`, 2); more are throttled.
    pub max_per_user: usize,
    /// Worker threads of the runtime queries run on, apart from request handling
    /// (`CATALOG_SQL_THREADS`, 2).
    pub threads: usize,
    /// Estimated resident memory the per-instance result cache may hold
    /// (`CATALOG_SQL_CACHE_MB`, 64).
    pub cache_bytes: usize,
}

const DEFAULT_SQL_MEMORY_BUDGET_BYTES: usize = 1024 << 20;

impl Default for SqlConfig {
    fn default() -> Self {
        let max_concurrent = 2;
        Self {
            limits: SqlLimits {
                max_rows: 1000,
                max_result_bytes: 8 << 20,
                timeout: Duration::from_secs(300),
                memory_limit_bytes: DEFAULT_SQL_MEMORY_BUDGET_BYTES / max_concurrent,
            },
            max_concurrent,
            max_per_user: 2,
            threads: 2,
            cache_bytes: DEFAULT_SQL_CACHE_BYTES,
        }
    }
}

impl SqlConfig {
    pub fn from_env() -> Result<Self> {
        let default = Self::default();
        let max_concurrent = positive_env("CATALOG_SQL_MAX_CONCURRENT", default.max_concurrent)?;
        let budget = megabytes_env(
            "CATALOG_SQL_MEMORY_BUDGET_MB",
            DEFAULT_SQL_MEMORY_BUDGET_BYTES,
        )?;
        Ok(Self {
            limits: SqlLimits {
                max_rows: positive_env("CATALOG_SQL_MAX_ROWS", default.limits.max_rows)?,
                max_result_bytes: megabytes_env(
                    "CATALOG_SQL_MAX_RESULT_MB",
                    default.limits.max_result_bytes,
                )?,
                timeout: Duration::from_secs(positive_env(
                    "CATALOG_SQL_TIMEOUT_SECS",
                    default.limits.timeout.as_secs(),
                )?),
                memory_limit_bytes: per_query_memory(budget, max_concurrent)?,
            },
            max_concurrent,
            max_per_user: positive_env("CATALOG_SQL_MAX_PER_USER", default.max_per_user)?,
            threads: positive_env("CATALOG_SQL_THREADS", default.threads)?,
            cache_bytes: megabytes_env("CATALOG_SQL_CACHE_MB", default.cache_bytes)?,
        })
    }
}

/// Each query's memory pool: an equal share of the budget per slot, at least one megabyte.
fn per_query_memory(budget_bytes: usize, max_concurrent: usize) -> Result<usize> {
    let share = budget_bytes / max_concurrent;
    anyhow::ensure!(
        share >= 1 << 20,
        "CATALOG_SQL_MEMORY_BUDGET_MB ({}) must allow at least 1 MB for each of the \
         CATALOG_SQL_MAX_CONCURRENT ({max_concurrent}) queries",
        budget_bytes >> 20
    );
    Ok(share)
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
        let cache_budget_bytes =
            megabytes_env("CATALOG_CACHE_MEMORY_LIMIT_MB", DEFAULT_CACHE_BUDGET_BYTES)?;
        Ok(Self {
            bind_addr,
            database_url: std::env::var("DATABASE_URL").context("DATABASE_URL must be set")?,
            db_schema: std::env::var("CATALOG_DB_SCHEMA")
                .ok()
                .filter(|s| !s.is_empty()),
            webui_dir: env_or("CATALOG_WEBUI_DIR", "frontend/dist"),
            verticals: parse_list(&std::env::var("CATALOG_VERTICALS").unwrap_or_default()),
            table_locations: parse_list(
                &std::env::var("CATALOG_TABLE_LOCATIONS").unwrap_or_default(),
            ),
            media_locations: parse_list(
                &std::env::var("CATALOG_MEDIA_LOCATIONS").unwrap_or_default(),
            ),
            cache_budget_bytes,
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

/// A positive number of megabytes from `name` in bytes, or `default_bytes` when unset.
fn megabytes_env(name: &str, default_bytes: usize) -> Result<usize> {
    positive_env(name, default_bytes >> 20)?
        .checked_mul(1 << 20)
        .with_context(|| format!("{name} is too large"))
}

#[cfg(test)]
mod tests {
    use super::{parse_list, per_query_memory};

    #[test]
    fn parse_list_trims_and_drops_empty_entries() {
        assert!(parse_list("").is_empty());
        assert!(parse_list(" , ").is_empty());
        assert_eq!(
            parse_list(" automotive, ,mining "),
            ["automotive", "mining"]
        );
    }

    #[test]
    fn memory_budget_is_split_across_slots() {
        assert_eq!(per_query_memory(3072 << 20, 10).unwrap(), (3072 << 20) / 10);
        assert_eq!(per_query_memory(4 << 20, 4).unwrap(), 1 << 20);
        let err = per_query_memory(3 << 20, 4).unwrap_err().to_string();
        assert!(err.contains("at least 1 MB"), "{err}");
    }
}
