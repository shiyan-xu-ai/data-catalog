//! Leader-only periodic sweep: run `catalog_store::sweep_root`, merge the result into the
//! current registry state with `apply_sweep_result`, and write the merged state back to the
//! `_catalog/registry` Lance path.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, Result};
use catalog_core::{apply_sweep_result, read_registry, write_registry, TableEntry};
use catalog_store::SweepConfig;
use tokio::task::JoinHandle;

use crate::leader::{is_leader, LeaderState};

/// Run a single sweep-merge-write cycle. Public so tests (and callers who want a one-shot
/// sweep instead of the loop) can invoke it directly.
pub async fn run_sweep_once(sweep_cfg: &SweepConfig, registry_path: &str) -> Result<()> {
    let swept = catalog_store::sweep_root(sweep_cfg)
        .await
        .context("sweep root")?;

    // The registry may not exist yet on the very first sweep.
    let mut current: HashMap<String, TableEntry> = match read_registry(registry_path).await {
        Ok(entries) => entries.into_iter().map(|e| (e.id.clone(), e)).collect(),
        Err(_) => HashMap::new(),
    };

    for entry in swept {
        apply_sweep_result(&mut current, entry);
    }

    let merged: Vec<TableEntry> = current.into_values().collect();
    write_registry(registry_path, &merged)
        .await
        .context("write merged registry")?;
    Ok(())
}

/// Spawn the periodic sweep loop. Only runs `run_sweep_once` while `leader_state` reports
/// leadership at the top of each tick; non-leader pods skip the sweep entirely (no
/// conflicting/duplicate writes).
pub fn spawn_sweep_loop(
    sweep_cfg: SweepConfig,
    registry_path: String,
    leader_state: LeaderState,
    interval: Duration,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            if is_leader(&leader_state) {
                if let Err(e) = run_sweep_once(&sweep_cfg, &registry_path).await {
                    tracing::error!(error = %e, "sweep cycle failed");
                }
            }
            tokio::time::sleep(interval).await;
        }
    })
}
