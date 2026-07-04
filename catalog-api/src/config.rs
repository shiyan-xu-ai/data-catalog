//! Env-var driven configuration for `catalog-api`.
//!
//! Leader election defaults to the real k8s Lease path (`CATALOG_LEADER_MODE=kube`, the
//! default). For local dev and tests where no cluster is available, set
//! `CATALOG_LEADER_MODE=forced-on` or `forced-off` to skip the k8s API entirely and report a
//! fixed leadership state.

use std::time::Duration;

use anyhow::{anyhow, Result};

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

fn env_duration_secs(name: &str, default_secs: u64) -> Duration {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(default_secs))
}

/// How leadership is determined.
#[derive(Debug, Clone)]
pub enum LeaderMode {
    /// Real `coordination.k8s.io/v1` Lease against the k8s API (production).
    Kube {
        namespace: String,
        lease_name: String,
        holder_identity: String,
    },
    /// Always report a fixed state, no k8s API call. Local dev / tests only.
    Forced(bool),
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub bind_addr: String,
    /// Bind address for the internal metrics/healthz server (separate port from main API).
    /// Scraped by Prometheus via the `metrics` port defined in the Service manifest.
    pub metrics_bind_addr: String,
    pub leader_mode: LeaderMode,
    /// How often the leader-election task re-ticks (acquire/renew attempt, or a forced
    /// no-op for `LeaderMode::Forced`).
    pub leader_tick_interval: Duration,
    /// Lease duration advertised to the k8s API when in `LeaderMode::Kube`.
    pub lease_duration: Duration,
    /// URI (plain path or `s3://bucket/prefix`) of the `_catalog/registry` Lance dataset.
    pub registry_path: String,
    /// URI of the `_catalog/ttl_audit` Lance dataset TTL `apply` appends deletion records to.
    pub ttl_audit_path: String,
    /// How often non-leader (and leader) pods re-read the registry into their local cache.
    pub registry_refresh_interval: Duration,
    /// URI of the sweep root (e.g. `s3://onroad-perception-datasets/scenario_dataset_export`
    /// or a local filesystem path for dev).
    pub sweep_root_uri: String,
    /// How often the leader runs a full sweep pass.
    pub sweep_interval: Duration,
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        let leader_mode = match env_or("CATALOG_LEADER_MODE", "kube").as_str() {
            "forced-on" => LeaderMode::Forced(true),
            "forced-off" => LeaderMode::Forced(false),
            "kube" => LeaderMode::Kube {
                namespace: env_or("CATALOG_LEASE_NAMESPACE", "default"),
                lease_name: env_or("CATALOG_LEASE_NAME", "catalog-api-leader"),
                holder_identity: env_or(
                    "CATALOG_POD_NAME",
                    &format!("catalog-api-{}", std::process::id()),
                ),
            },
            other => {
                return Err(anyhow!(
                    "invalid CATALOG_LEADER_MODE={other} (expected kube|forced-on|forced-off)"
                ))
            }
        };

        let leader_tick_interval = env_duration_secs("CATALOG_LEADER_TICK_INTERVAL_SECS", 10);
        let lease_duration = env_duration_secs("CATALOG_LEASE_DURATION_SECS", 30);
        // The tick interval must be shorter than the lease duration: the elector renews once
        // per tick, so an interval >= lease_duration means the lease can expire between renewals
        // and leadership flaps every cycle. The deadline policy in `run_leader_election` also
        // bounds each tick by one interval, which must stay under the lease to be meaningful.
        if leader_tick_interval >= lease_duration {
            return Err(anyhow!(
                "CATALOG_LEADER_TICK_INTERVAL_SECS ({}s) must be less than \
                 CATALOG_LEASE_DURATION_SECS ({}s)",
                leader_tick_interval.as_secs(),
                lease_duration.as_secs()
            ));
        }

        Ok(Self {
            bind_addr: env_or("CATALOG_BIND_ADDR", "0.0.0.0:8080"),
            metrics_bind_addr: env_or("CATALOG_METRICS_BIND_ADDR", "0.0.0.0:9090"),
            leader_mode,
            leader_tick_interval,
            lease_duration,
            registry_path: env_or("CATALOG_REGISTRY_PATH", "_catalog/registry"),
            ttl_audit_path: env_or("CATALOG_TTL_AUDIT_PATH", "_catalog/ttl_audit"),
            registry_refresh_interval: env_duration_secs(
                "CATALOG_REGISTRY_REFRESH_INTERVAL_SECS",
                5,
            ),
            sweep_root_uri: env_or(
                "CATALOG_SWEEP_ROOT_URI",
                "s3://onroad-perception-datasets/scenario_dataset_export",
            ),
            // Default per findings.md ("~30 min full pass"); override for tests/local dev.
            sweep_interval: env_duration_secs("CATALOG_SWEEP_INTERVAL_SECS", 1800),
        })
    }
}
