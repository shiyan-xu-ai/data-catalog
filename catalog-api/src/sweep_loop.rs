//! Leader-only periodic sweep: run `catalog_store::sweep_root`, merge the result into the
//! current registry state with `apply_sweep_result`, and write the merged state back to the
//! `_catalog/registry` Lance path.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use catalog_core::{apply_sweep_result, read_registry, write_registry, TableEntry};
use catalog_store::SweepConfig;
use tokio::task::JoinHandle;

use crate::leader::{is_leader, LeaderState};
use crate::registry_lock::RegistryWriteLock;

/// Run a single sweep-merge-write cycle. Public so tests (and callers who want a one-shot
/// sweep instead of the loop) can invoke it directly.
///
/// `leader_state` is re-checked immediately before the registry write (not just once at the
/// top of the calling loop's tick): a full sweep over many tables can take long enough to
/// outlive an actual leadership loss (lease expiry / takeover by a new leader), and writing
/// the registry after leadership has already flipped away would race the new leader's own
/// writes. If leadership was lost mid-sweep, the write is skipped and the sweep result is
/// discarded -- the new leader's next sweep will pick everything back up.
///
/// `write_lock` is the SAME `RegistryWriteLock` the REST API mutation handlers use
/// (`ApiState::write_lock`). It is held across the whole read-registry -> merge ->
/// leader-recheck -> write-registry critical section below, so this sweep's
/// read-modify-write can never interleave with a concurrent API mutation's own
/// read-modify-write of the same path -- closing the lost-update race a Phase 5 review
/// found (a sweep reading a stale registry, then writing it back after a concurrent
/// `DeclareTable`/`protect` call had already committed, silently reverting it). This matters
/// starting Phase 6 because TTL apply consumes the `protected` flag: a silently-reverted
/// `protect` would otherwise make a hard-delete irreversible.
pub async fn run_sweep_once(
    sweep_cfg: &SweepConfig,
    registry_path: &str,
    leader_state: &LeaderState,
    write_lock: &RegistryWriteLock,
) -> Result<()> {
    run_sweep_once_inner(sweep_cfg, registry_path, leader_state, write_lock, None).await
}

/// Shared implementation. `after_read` (test-only) is notified exactly once, immediately
/// after the registry has been read but before it is written back, while `write_lock` is
/// still held -- lets a regression test deterministically prove that a concurrent writer
/// attempting to acquire the same lock is blocked for the entire critical section, not just
/// part of it. Production callers pass `None`.
async fn run_sweep_once_inner(
    sweep_cfg: &SweepConfig,
    registry_path: &str,
    leader_state: &LeaderState,
    write_lock: &RegistryWriteLock,
    after_read: Option<Arc<tokio::sync::Notify>>,
) -> Result<()> {
    let swept = catalog_store::sweep_root(sweep_cfg)
        .await
        .context("sweep root")?;

    let _guard = write_lock.lock().await;

    // The registry may not exist yet on the very first sweep.
    let mut current: HashMap<String, TableEntry> = match read_registry(registry_path).await {
        Ok(entries) => entries.into_iter().map(|e| (e.id.clone(), e)).collect(),
        Err(_) => HashMap::new(),
    };

    if let Some(notify) = &after_read {
        notify.notify_one();
        // Widen the window (test-only): give a concurrent writer racing on `write_lock` a
        // real chance to attempt (and correctly block on) the lock before this critical
        // section finishes -- see `sweep_write_and_api_write_are_mutually_exclusive` below.
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

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
/// before writing, in case leadership is lost partway through a long sweep. `write_lock` is
/// the same lock shared with the REST API mutation handlers (see `run_sweep_once`'s docs).
pub fn spawn_sweep_loop(
    sweep_cfg: SweepConfig,
    registry_path: String,
    leader_state: LeaderState,
    write_lock: RegistryWriteLock,
    interval: Duration,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            if is_leader(&leader_state) {
                if let Err(e) =
                    run_sweep_once(&sweep_cfg, &registry_path, &leader_state, &write_lock).await
                {
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
    use crate::api::{api_router, ApiState};
    use crate::registry_lock::new_registry_write_lock;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use object_store::local::LocalFileSystem;
    use object_store::path::Path as ObjPath;
    use std::sync::atomic::AtomicBool;
    use tower::ServiceExt;

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
        let write_lock = new_registry_write_lock();

        // Leadership already false by the time run_sweep_once is called -- simulates the
        // "lost leadership mid-sweep" window the re-check exists to close.
        let leader_state: LeaderState = Arc::new(AtomicBool::new(false));
        run_sweep_once(&sweep_cfg, &registry_path, &leader_state, &write_lock)
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
        let write_lock = new_registry_write_lock();

        let leader_state: LeaderState = Arc::new(AtomicBool::new(true));
        run_sweep_once(&sweep_cfg, &registry_path, &leader_state, &write_lock)
            .await
            .expect("sweep-merge-write should succeed");

        assert!(
            read_registry(&registry_path).await.is_ok(),
            "registry should have been written while still leader"
        );
    }

    /// Regression test for the REQUIRED-BEFORE-PHASE-6 hardening item (status.md): before this
    /// fix, the sweep loop's read-modify-write held NO lock at all, so it could read the
    /// registry, then (after a concurrent API mutation committed a change in between) write its
    /// own stale copy back on top, silently reverting the API's write. Now both writers share
    /// the SAME `RegistryWriteLock`, so their critical sections can never interleave.
    ///
    /// This drives the REAL `run_sweep_once` (via the test-only `after_read` pause hook) and the
    /// REAL `declare_table` HTTP handler (via `api_router`) concurrently against the same
    /// registry path and the same lock: the sweep reads a stale `owner: None`, then -- while
    /// still holding the lock -- sleeps; concurrently we fire a `DeclareTable` call that sets
    /// `owner: raymond` and must block on the same lock until the sweep's write completes. If
    /// the fix works, `DeclareTable`'s own read only happens AFTER the sweep's write, so its
    /// write lands last and the owner survives. Without the shared lock, this exact race is the
    /// one the Phase 5 reviewer traced as a genuine lost update.
    #[tokio::test]
    async fn sweep_write_and_api_write_are_mutually_exclusive() {
        let sweep_root = tempfile::tempdir().unwrap(); // empty: sweep finds no tables on S3
        let sweep_cfg = empty_sweep_config(sweep_root.path());
        let registry_dir = tempfile::tempdir().unwrap();
        let registry_path = registry_dir
            .path()
            .join("registry.lance")
            .to_str()
            .unwrap()
            .to_string();

        let seed = vec![TableEntry {
            id: "smoke_test".to_string(),
            name: "smoke_test".to_string(),
            namespace: catalog_core::Namespace::new(["scenario_dataset_export"]),
            root_location: "s3://bucket/smoke_test".to_string(),
            owner: None,
            ttl_policy: None,
            last_swept: None,
            versions: Vec::new(),
            aux_latest: Vec::new(),
        }];
        write_registry(&registry_path, &seed).await.unwrap();

        let write_lock = new_registry_write_lock();
        let leader_state: LeaderState = Arc::new(AtomicBool::new(true));
        let after_read = Arc::new(tokio::sync::Notify::new());

        let sweep_task = {
            let sweep_cfg = sweep_cfg.clone();
            let registry_path = registry_path.clone();
            let leader_state = leader_state.clone();
            let write_lock = write_lock.clone();
            let after_read = after_read.clone();
            tokio::spawn(async move {
                run_sweep_once_inner(
                    &sweep_cfg,
                    &registry_path,
                    &leader_state,
                    &write_lock,
                    Some(after_read),
                )
                .await
                .expect("sweep should not fail");
            })
        };

        // Wait until the sweep has read the (stale, owner=None) registry and is holding
        // write_lock through its artificial pause, before firing the racing API mutation.
        after_read.notified().await;

        let cache = Arc::new(tokio::sync::RwLock::new(seed));
        let state = ApiState::new(
            registry_path.clone(),
            cache,
            leader_state.clone(),
            write_lock.clone(),
            sweep_cfg.clone(),
            registry_dir
                .path()
                .join("ttl_audit.lance")
                .to_str()
                .unwrap()
                .to_string(),
        );
        let app = api_router(state);
        let declare_task = tokio::spawn(async move {
            app.oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/v1/table/smoke_test")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"owner":"raymond"}"#))
                    .unwrap(),
            )
            .await
            .unwrap()
        });

        sweep_task.await.unwrap();
        let resp = declare_task.await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let final_registry = read_registry(&registry_path).await.unwrap();
        let entry = final_registry
            .iter()
            .find(|e| e.id == "smoke_test")
            .expect("table must still be present");
        assert_eq!(
            entry.owner.as_deref(),
            Some("raymond"),
            "the API-set owner must survive the racing sweep write -- the shared \
             RegistryWriteLock must prevent the sweep's stale read-then-write from clobbering it"
        );
    }
}
