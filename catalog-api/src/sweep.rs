//! Sweep: refresh the derived registry snapshot for the registered tables.
//!
//! The catalog is a curated allowlist, not a mirror of the bucket: only tables an operator has
//! registered (an authored overlay object exists for them, written by `DeclareTable`) are swept.
//! The sweep never enumerates the whole root, so unregistered tables on S3 are ignored.
//!
//! On Cloud Run this runs request-scoped (triggered by Cloud Scheduler hitting
//! `/internal/jobs/sweep`), not as a background loop — CPU is throttled between requests, so a
//! `tokio`-spawned loop would freeze. `run_sweep` is idempotent and safe under Scheduler
//! double-fire/retry: the snapshot is a complete derived state written last-wins, with no lock
//! or leader (only the sweep writes it). Authored state (owner/ttl_policy/protected) lives in
//! the overlay and is never touched here.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use anyhow::{Context, Result};
use catalog_core::{read_registry, write_registry, TableEntry};
use catalog_store::{MetaStore, SweepConfig};
use object_store::path::Path as ObjPath;
use serde::Serialize;

/// Summary of one sweep pass, returned to the scheduler endpoint and logged.
#[derive(Debug, Clone, Serialize)]
pub struct SweepReport {
    pub tables_checked: u64,
    pub failed_tables: u64,
    pub duration_secs: f64,
}

/// Run one sweep: for each registered table, re-derive its version set from S3 and write the
/// whole derived snapshot (registered tables only), reconcile stale overlay markers, and
/// best-effort prune old registry versions. A registered table that fails to sweep is isolated
/// (kept from the prior snapshot, counted, logged) rather than dropped. A registered table with
/// no directory on S3 yet sweeps cleanly to zero versions (it shows as a stub until data lands).
pub async fn run_sweep(
    sweep_cfg: &SweepConfig,
    registry_path: &str,
    meta: &MetaStore,
) -> Result<SweepReport> {
    let started = Instant::now();

    // The registered set = the tables that have an authored overlay. Only these are swept.
    let registered = meta.list_meta().await.context("list registered tables")?;

    // Prior snapshot is only needed to isolate a registered table that fails this pass.
    let prior: HashMap<String, TableEntry> = read_registry(registry_path)
        .await
        .context("read prior snapshot")?
        .unwrap_or_default()
        .into_iter()
        .map(|e| (e.id.clone(), e))
        .collect();

    // Rebuild the snapshot from scratch as exactly the registered set: a table removed from the
    // registered set (deregistered) is dropped here, keeping the snapshot an allowlist. No
    // authored fields are written — those live in the overlay and are merged in at read time.
    let mut by_id: HashMap<String, TableEntry> = HashMap::with_capacity(registered.len());
    let mut failed_tables = 0u64;
    for table_id in registered.keys() {
        match catalog_store::sweep_table(sweep_cfg, table_id, &table_path(sweep_cfg, table_id))
            .await
        {
            Ok(entry) => {
                by_id.insert(table_id.clone(), entry);
            }
            Err(e) => {
                failed_tables += 1;
                tracing::warn!(
                    table = %table_id,
                    error = %e,
                    "sweep: skipping registered table that failed this cycle"
                );
                // Isolation: keep the prior derived entry rather than dropping the table on a
                // transient failure.
                if let Some(prev) = prior.get(table_id) {
                    by_id.insert(table_id.clone(), prev.clone());
                }
            }
        }
    }
    let tables_checked = registered.len() as u64;

    // Reconcile stale overlay markers to the new snapshot truth: a `protected`/`deleting` entry
    // for a version no longer in the snapshot (TTL-deleted then swept out, or gone out of band)
    // is pruned. This clears the tombstones a TTL apply leaves behind once the version is truly
    // gone. Best-effort — a failure here doesn't fail the sweep.
    if let Err(e) = reconcile_overlays(meta, &by_id).await {
        tracing::warn!(error = %e, "overlay reconciliation failed (non-fatal)");
    }

    let merged: Vec<TableEntry> = by_id.into_values().collect();

    // Last-wins whole-snapshot write — no coordination needed for recomputable derived state.
    write_registry(registry_path, &merged)
        .await
        .context("write snapshot")?;

    // Bounded growth: prune registry manifest versions older than a day (each sweep adds one).
    if let Err(e) = catalog_core::cleanup_registry(registry_path, chrono::Duration::days(1)).await {
        tracing::warn!(error = %e, "registry version cleanup failed (non-fatal)");
    }

    let report = SweepReport {
        tables_checked,
        failed_tables,
        duration_secs: started.elapsed().as_secs_f64(),
    };
    tracing::info!(
        tables_checked = report.tables_checked,
        failed_tables = report.failed_tables,
        duration_secs = report.duration_secs,
        "sweep complete"
    );
    Ok(report)
}

/// The object-store path of one table's directory: `<root_path>/<table_id>`. `ObjPath::from`
/// normalizes empty/leading segments, so an empty root (local dev / tests) yields just the id.
fn table_path(sweep_cfg: &SweepConfig, table_id: &str) -> ObjPath {
    let root = sweep_cfg.root_path.as_ref().trim_end_matches('/');
    if root.is_empty() {
        ObjPath::from(table_id)
    } else {
        ObjPath::from(format!("{root}/{table_id}"))
    }
}

/// Prune overlay `protected`/`deleting` version ids that no longer exist in the snapshot for
/// their table (TTL-deleted-and-swept, or removed out of band). Leaves owner/ttl_policy alone.
async fn reconcile_overlays(
    meta: &MetaStore,
    snapshot: &HashMap<String, TableEntry>,
) -> Result<()> {
    for (table_id, overlay) in meta.list_meta().await? {
        if overlay.protected.is_empty() && overlay.deleting.is_empty() {
            continue;
        }
        let live: HashSet<&str> = snapshot
            .get(&table_id)
            .map(|e| e.versions.iter().map(|v| v.version_id.as_str()).collect())
            .unwrap_or_default();
        let has_stale = overlay
            .protected
            .iter()
            .chain(overlay.deleting.iter())
            .any(|vid| !live.contains(vid.as_str()));
        if !has_stale {
            continue;
        }
        // Rebuild the live set inside the closure (owned) since the CAS may retry.
        let live_ids: HashSet<String> = snapshot
            .get(&table_id)
            .map(|e| e.versions.iter().map(|v| v.version_id.clone()).collect())
            .unwrap_or_default();
        meta.mutate_meta(&table_id, |m| {
            m.protected.retain(|vid| live_ids.contains(vid));
            m.deleting.retain(|vid| live_ids.contains(vid));
        })
        .await?;
    }
    Ok(())
}
