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

/// Shared implementation. `after_read` is a **test seam**: when `Some`, it is notified exactly
/// once immediately after the registry has been read but before it is written back, while
/// `write_lock` is still held -- letting the mutual-exclusion regression test deterministically
/// prove a concurrent writer is blocked for the entire critical section. Production callers use
/// `run_sweep_once` (which passes `None`); this is `pub` only so that regression test can live
/// in the integration tier (`tests/sweep_loop_test.rs`) rather than inline.
pub async fn run_sweep_once_inner(
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
        // real chance to attempt (and correctly block on) the lock before this critical section
        // finishes -- see `sweep_write_and_api_write_are_mutually_exclusive` in
        // `tests/sweep_loop_test.rs`.
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

    // Pure unit test for the sweep-due gate. The IO/HTTP tests that exercise `run_sweep_once`
    // and the sweep-vs-API mutual exclusion live in the integration tier
    // (`tests/sweep_loop_test.rs`).
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
}
