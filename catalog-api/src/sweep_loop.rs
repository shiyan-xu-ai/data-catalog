//! Leader-only periodic sweep: run `catalog_store::sweep_root`, merge the result into the
//! current registry state with `apply_sweep_result`, and write the merged state back to the
//! `_catalog/registry` Lance path.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use catalog_core::{
    apply_sweep_result, read_registry, ttl_eligible_versions, write_registry, TableEntry,
};
use catalog_store::SweepConfig;
use tokio::task::JoinHandle;

use crate::leader::{is_leader, LeaderState};
use crate::metrics;
use crate::registry_lock::RegistryWriteLock;

/// Shared state tracking the instant the last successful sweep write completed.
/// `None` means no sweep has completed yet in this process.
pub type LastSweepAt = Arc<tokio::sync::Mutex<Option<Instant>>>;

/// Create a new, initially-empty `LastSweepAt` tracker.
pub fn new_last_sweep_at() -> LastSweepAt {
    Arc::new(tokio::sync::Mutex::new(None))
}

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
///
/// On success, `last_sweep_at` is updated to `Instant::now()` so the staleness gauge updater
/// can report time elapsed since the last completed write.
pub async fn run_sweep_once(
    sweep_cfg: &SweepConfig,
    registry_path: &str,
    leader_state: &LeaderState,
    write_lock: &RegistryWriteLock,
    last_sweep_at: &LastSweepAt,
) -> Result<()> {
    run_sweep_once_inner(
        sweep_cfg,
        registry_path,
        leader_state,
        write_lock,
        last_sweep_at,
        None,
    )
    .await
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
    last_sweep_at: &LastSweepAt,
    after_read: Option<Arc<tokio::sync::Notify>>,
) -> Result<()> {
    let cycle_start = Instant::now();

    // Record the discovery LIST outcome AFTER it runs, reflecting the real result rather than
    // an unconditional success stamped before the call.
    let swept = match catalog_store::sweep_root(sweep_cfg).await {
        Ok(outcome) => {
            metrics::record_s3_op("sweep_list", true);
            outcome
        }
        Err(e) => {
            metrics::record_s3_op("sweep_list", false);
            return Err(e).context("sweep root");
        }
    };
    metrics::record_sweep_failures(swept.failed_tables);

    let tables_checked = swept.tables.len() as u64;

    let _guard = write_lock.lock().await;

    // The registry may not exist yet on the very first sweep (`Ok(None)`) -- start from an
    // empty map in that case only. A real read error aborts the whole cycle: merging the
    // freshly-swept tables into an empty map and writing that back (via `Overwrite`) would
    // silently drop every table's API-assigned owner/ttl_policy and reset every version's
    // `protected` flag. Discarding this cycle is recoverable; overwriting the registry is not.
    let mut current: HashMap<String, TableEntry> = match read_registry(registry_path)
        .await
        .context("read registry for sweep merge")?
    {
        Some(entries) => entries.into_iter().map(|e| (e.id.clone(), e)).collect(),
        None => HashMap::new(),
    };

    if let Some(notify) = &after_read {
        notify.notify_one();
        // Widen the window (test-only): give a concurrent writer racing on `write_lock` a
        // real chance to attempt (and correctly block on) the lock before this critical
        // section finishes -- see `sweep_write_and_api_write_are_mutually_exclusive` below.
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    for entry in swept.tables {
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

    // Maintain the reclaimable-bytes gauge from ground truth here (leader-only), so it is a
    // stable total across all tables rather than a per-table value clobbered by whichever
    // dry-run ran last. Cheap: arithmetic over the in-memory versions just written.
    metrics::set_ttl_reclaimable_bytes(total_reclaimable_bytes(&merged, chrono::Utc::now()));

    // Record completion time so the staleness gauge updater can track elapsed seconds.
    *last_sweep_at.lock().await = Some(Instant::now());

    let elapsed = cycle_start.elapsed();
    metrics::record_sweep_cycle(tables_checked, elapsed);

    Ok(())
}

/// Sum of logical reclaimable bytes across all tables under their current TTL policies, as of
/// `now`. A table with no policy contributes nothing (`ttl_eligible_versions` returns empty).
fn total_reclaimable_bytes(tables: &[TableEntry], now: chrono::DateTime<chrono::Utc>) -> u64 {
    tables
        .iter()
        .map(|t| {
            let policy = t.ttl_policy.unwrap_or_default();
            ttl_eligible_versions(&policy, &t.versions, now)
                .iter()
                .map(|v| v.storage_bytes_total)
                .sum::<u64>()
        })
        .sum()
}

/// Whether a sweep is due: never swept yet (a freshly-elected leader sweeps right away), or at
/// least `interval` has elapsed since the last successful sweep. Pure so the gating is
/// unit-testable with synthetic `Instant`s.
fn sweep_due(last_sweep_at: Option<Instant>, now: Instant, interval: Duration) -> bool {
    last_sweep_at.is_none_or(|last| now.duration_since(last) >= interval)
}

/// Spawn the periodic sweep loop. Only runs `run_sweep_once` while `leader_state` reports
/// leadership; non-leader pods skip the sweep entirely (no conflicting/duplicate writes).
///
/// The loop polls faster than `interval` and gates the actual sweep on `sweep_due`, so a
/// newly-elected leader starts sweeping within one poll rather than waiting up to a full
/// `interval` (default 30 min). `run_sweep_once` itself re-checks `leader_state` again right
/// before writing, in case leadership is lost partway through a long sweep. `write_lock` is the
/// same lock shared with the REST API mutation handlers (see `run_sweep_once`'s docs).
pub fn spawn_sweep_loop(
    sweep_cfg: SweepConfig,
    registry_path: String,
    leader_state: LeaderState,
    write_lock: RegistryWriteLock,
    last_sweep_at: LastSweepAt,
    interval: Duration,
) -> JoinHandle<()> {
    // Poll cadence: fast enough to react to a leadership change promptly, but never above the
    // sweep interval (short intervals in tests keep their original cadence).
    let poll_interval = interval.min(Duration::from_secs(15));
    tokio::spawn(async move {
        loop {
            if is_leader(&leader_state)
                && sweep_due(*last_sweep_at.lock().await, Instant::now(), interval)
            {
                if let Err(e) = run_sweep_once(
                    &sweep_cfg,
                    &registry_path,
                    &leader_state,
                    &write_lock,
                    &last_sweep_at,
                )
                .await
                {
                    tracing::error!(error = %e, "sweep cycle failed");
                }
            }
            tokio::time::sleep(poll_interval).await;
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

    #[test]
    fn sweep_due_fires_immediately_when_never_swept_then_gates_on_the_interval() {
        let now = Instant::now();
        let interval = Duration::from_secs(60);
        // Never swept (e.g. a freshly-elected leader): due immediately.
        assert!(sweep_due(None, now, interval));
        // Swept recently: not due yet.
        assert!(!sweep_due(
            Some(now - Duration::from_secs(30)),
            now,
            interval
        ));
        // Interval elapsed since the last sweep: due again.
        assert!(sweep_due(
            Some(now - Duration::from_secs(90)),
            now,
            interval
        ));
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
        let last_sweep_at = new_last_sweep_at();

        // Leadership already false by the time run_sweep_once is called -- simulates the
        // "lost leadership mid-sweep" window the re-check exists to close.
        let leader_state: LeaderState = Arc::new(AtomicBool::new(false));
        run_sweep_once(
            &sweep_cfg,
            &registry_path,
            &leader_state,
            &write_lock,
            &last_sweep_at,
        )
        .await
        .expect("sweep-merge should not fail even when the write is skipped");

        // No write ever happened: the registry dataset was never created.
        assert!(
            read_registry(&registry_path).await.unwrap().is_none(),
            "registry should not exist -- the write must have been skipped"
        );
        // last_sweep_at must remain None when the write was skipped.
        assert!(
            last_sweep_at.lock().await.is_none(),
            "last_sweep_at must stay None when registry write was skipped"
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
        let last_sweep_at = new_last_sweep_at();

        let leader_state: LeaderState = Arc::new(AtomicBool::new(true));
        run_sweep_once(
            &sweep_cfg,
            &registry_path,
            &leader_state,
            &write_lock,
            &last_sweep_at,
        )
        .await
        .expect("sweep-merge-write should succeed");

        assert!(
            read_registry(&registry_path).await.unwrap().is_some(),
            "registry should have been written while still leader"
        );
        // last_sweep_at must be populated after a successful write.
        assert!(
            last_sweep_at.lock().await.is_some(),
            "last_sweep_at must be set after a successful sweep write"
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

        let last_sweep_at = new_last_sweep_at();
        let sweep_task = {
            let sweep_cfg = sweep_cfg.clone();
            let registry_path = registry_path.clone();
            let leader_state = leader_state.clone();
            let write_lock = write_lock.clone();
            let after_read = after_read.clone();
            let last_sweep_at = last_sweep_at.clone();
            tokio::spawn(async move {
                run_sweep_once_inner(
                    &sweep_cfg,
                    &registry_path,
                    &leader_state,
                    &write_lock,
                    &last_sweep_at,
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

        let final_registry = read_registry(&registry_path)
            .await
            .unwrap()
            .expect("registry exists after the sweep write");
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
