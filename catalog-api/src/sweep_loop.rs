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
///
/// `leader_state` is re-checked immediately before the registry write (not just once at the
/// top of the calling loop's tick): a full sweep over many tables can take long enough to
/// outlive an actual leadership loss (lease expiry / takeover by a new leader), and writing
/// the registry after leadership has already flipped away would race the new leader's own
/// writes. If leadership was lost mid-sweep, the write is skipped and the sweep result is
/// discarded -- the new leader's next sweep will pick everything back up.
pub async fn run_sweep_once(
    sweep_cfg: &SweepConfig,
    registry_path: &str,
    leader_state: &LeaderState,
) -> Result<()> {
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

    if !is_leader(leader_state) {
        tracing::warn!("leadership lost mid-sweep; skipping registry write");
        return Ok(());
    }

    let merged: Vec<TableEntry> = current.into_values().collect();
    write_registry(registry_path, &merged)
        .await
        .context("write merged registry")?;
    Ok(())
}

/// Spawn the periodic sweep loop. Only runs `run_sweep_once` while `leader_state` reports
/// leadership at the top of each tick; non-leader pods skip the sweep entirely (no
/// conflicting/duplicate writes). `run_sweep_once` itself re-checks `leader_state` again right
/// before writing, in case leadership is lost partway through a long sweep.
pub fn spawn_sweep_loop(
    sweep_cfg: SweepConfig,
    registry_path: String,
    leader_state: LeaderState,
    interval: Duration,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            if is_leader(&leader_state) {
                if let Err(e) = run_sweep_once(&sweep_cfg, &registry_path, &leader_state).await {
                    tracing::error!(error = %e, "sweep cycle failed");
                }
            }
            tokio::time::sleep(interval).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use object_store::local::LocalFileSystem;
    use object_store::path::Path as ObjPath;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    fn empty_sweep_config(tmp: &std::path::Path) -> SweepConfig {
        let store = Arc::new(LocalFileSystem::new_with_prefix(tmp).unwrap());
        SweepConfig::new(store, ObjPath::from(""), tmp.to_str().unwrap().to_string())
    }

    #[tokio::test]
    async fn run_sweep_once_skips_the_write_if_leadership_is_already_lost() {
        let sweep_root = tempfile::tempdir().unwrap();
        let sweep_cfg = empty_sweep_config(sweep_root.path());
        let registry_dir = tempfile::tempdir().unwrap();
        let registry_path = registry_dir
            .path()
            .join("registry.lance")
            .to_str()
            .unwrap()
            .to_string();

        // Leadership already false by the time run_sweep_once is called -- simulates the
        // "lost leadership mid-sweep" window the re-check exists to close.
        let leader_state: LeaderState = Arc::new(AtomicBool::new(false));
        run_sweep_once(&sweep_cfg, &registry_path, &leader_state)
            .await
            .expect("sweep-merge should not fail even when the write is skipped");

        // No write ever happened: the registry dataset was never created.
        assert!(
            read_registry(&registry_path).await.is_err(),
            "registry should not exist -- the write must have been skipped"
        );
    }

    #[tokio::test]
    async fn run_sweep_once_writes_when_still_leader_at_write_time() {
        let sweep_root = tempfile::tempdir().unwrap();
        let sweep_cfg = empty_sweep_config(sweep_root.path());
        let registry_dir = tempfile::tempdir().unwrap();
        let registry_path = registry_dir
            .path()
            .join("registry.lance")
            .to_str()
            .unwrap()
            .to_string();

        let leader_state: LeaderState = Arc::new(AtomicBool::new(true));
        run_sweep_once(&sweep_cfg, &registry_path, &leader_state)
            .await
            .expect("sweep-merge-write should succeed");

        assert!(
            read_registry(&registry_path).await.is_ok(),
            "registry should have been written while still leader"
        );
    }
}
