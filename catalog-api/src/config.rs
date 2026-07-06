//! Env-var driven configuration for `catalog-api` on Apps Platform (Cloud Run).

use std::time::Duration;

use anyhow::{anyhow, Result};

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

/// Parse a duration-in-seconds env var, falling back to `default_secs` when unset. A set-but-
/// invalid value or `0` is a hard error rather than a silent fallback.
fn env_duration_secs(name: &str, default_secs: u64) -> Result<Duration> {
    let secs = match std::env::var(name) {
        Ok(v) => v.parse::<u64>().map_err(|_| {
            anyhow!("{name} must be a positive integer number of seconds, got {v:?}")
        })?,
        Err(_) => default_secs,
    };
    if secs == 0 {
        return Err(anyhow!("{name} must be greater than 0 seconds"));
    }
    Ok(Duration::from_secs(secs))
}

/// Parse a positive-integer env var, falling back to `default` when unset. A set-but-invalid
/// value or `0` is a hard error rather than a silent fallback (same contract as
/// `env_duration_secs`).
fn env_positive_usize(name: &str, default: usize) -> Result<usize> {
    let n = match std::env::var(name) {
        Ok(v) => v
            .parse::<usize>()
            .map_err(|_| anyhow!("{name} must be a positive integer, got {v:?}"))?,
        Err(_) => default,
    };
    if n == 0 {
        return Err(anyhow!("{name} must be greater than 0"));
    }
    Ok(n)
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    /// Address the HTTP server binds. Cloud Run injects `PORT`; default `0.0.0.0:8080`.
    pub bind_addr: String,
    /// URI (plain path or `s3://bucket/prefix`) of the `_catalog/registry` derived snapshot.
    pub registry_path: String,
    /// URI of the `_catalog/ttl_audit` Lance dataset.
    pub ttl_audit_path: String,
    /// URI base of the authored overlay objects. `memory` (dev/tests, non-persistent),
    /// `s3://bucket/_catalog/meta` (prod / MinIO local). A plain filesystem path does NOT work:
    /// `LocalFileSystem` lacks conditional writes.
    pub meta_base_uri: String,
    /// Path to the deployment-scoped catalog config (region, buckets, namespaces, admins).
    pub catalog_config_path: String,
    /// How long the merged read cache may be served before revalidation.
    pub cache_ttl: Duration,
    /// How many versions the sync processes concurrently (in-flight LISTs + Lance opens).
    pub sync_concurrency: usize,
    /// Which versions get the expensive Lance stats (`all` | `latest` | `none`).
    pub sync_deep_stats: catalog_store::DeepStats,
    /// Directory holding the built frontend (`dist/`). Served at `/` when it contains an
    /// `index.html`; absent → API-only.
    pub webui_dir: String,
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        // Cloud Run sets PORT; honor it, else CATALOG_BIND_ADDR, else the default.
        let bind_addr = match std::env::var("PORT") {
            Ok(port) => format!("0.0.0.0:{port}"),
            Err(_) => env_or("CATALOG_BIND_ADDR", "0.0.0.0:8080"),
        };

        Ok(Self {
            bind_addr,
            registry_path: env_or("CATALOG_REGISTRY_PATH", "_catalog/registry"),
            ttl_audit_path: env_or("CATALOG_TTL_AUDIT_PATH", "_catalog/ttl_audit"),
            meta_base_uri: env_or("CATALOG_META_BASE_URI", "memory"),
            catalog_config_path: env_or("CATALOG_CONFIG_PATH", "catalog-config.yaml"),
            cache_ttl: env_duration_secs("CATALOG_CACHE_TTL_SECS", 5)?,
            sync_concurrency: env_positive_usize(
                "CATALOG_SYNC_CONCURRENCY",
                catalog_store::DEFAULT_SYNC_CONCURRENCY,
            )?,
            sync_deep_stats: match std::env::var("CATALOG_SYNC_DEEP_STATS") {
                Ok(v) => v.parse()?,
                Err(_) => catalog_store::DeepStats::default(),
            },
            webui_dir: env_or("CATALOG_WEBUI_DIR", "frontend/dist"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_duration_secs_defaults_parses_and_rejects_invalid_and_zero() {
        let name = "CATALOG_TEST_ENV_DURATION_SECS";
        // SAFETY: single-threaded within this test; the unique name avoids clashing with any
        // other test that mutates the environment.
        unsafe {
            std::env::remove_var(name);
        }
        assert_eq!(env_duration_secs(name, 7).unwrap(), Duration::from_secs(7));
        unsafe {
            std::env::set_var(name, "42");
        }
        assert_eq!(env_duration_secs(name, 7).unwrap(), Duration::from_secs(42));
        unsafe {
            std::env::set_var(name, "notanumber");
        }
        assert!(env_duration_secs(name, 7).is_err());
        unsafe {
            std::env::set_var(name, "0");
        }
        assert!(env_duration_secs(name, 7).is_err());
        unsafe {
            std::env::remove_var(name);
        }
    }
}
