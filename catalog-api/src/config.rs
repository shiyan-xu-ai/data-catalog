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
    /// URI of the sweep root.
    pub sweep_root_uri: String,
    /// How long the merged read cache may be served before revalidation.
    pub cache_ttl: Duration,
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
            sweep_root_uri: env_or(
                "CATALOG_SWEEP_ROOT_URI",
                "s3://onroad-perception-datasets/scenario_dataset_export",
            ),
            cache_ttl: env_duration_secs("CATALOG_CACHE_TTL_SECS", 5)?,
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
