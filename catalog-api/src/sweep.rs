//! Sweep: discover S3 tables/versions and write the derived registry snapshot.
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
use catalog_core::{apply_sweep_result, read_registry, write_registry, TableEntry};
use catalog_store::{MetaStore, SweepConfig};
use serde::Serialize;

/// Summary of one sweep pass, returned to the scheduler endpoint and logged.
#[derive(Debug, Clone, Serialize)]
pub struct SweepReport {
    pub tables_checked: u64,
    pub failed_tables: u64,
    pub duration_secs: f64,
}

/// Run one sweep: list the root, reconcile the derived snapshot to the swept set, write it,
/// reconcile stale overlay markers, and best-effort prune old registry versions. Tables that
/// fail to sweep are isolated (kept from the prior snapshot, counted, logged) rather than
/// dropped.
pub async fn run_sweep(
    sweep_cfg: &SweepConfig,
    registry_path: &str,
    meta: &MetaStore,
) -> Result<SweepReport> {
    let started = Instant::now();

    let outcome = catalog_store::sweep_root(sweep_cfg)
        .await
        .context("sweep root")?;
    let tables_checked = outcome.tables.len() as u64;

    // Merge the freshly-swept (derived-only) tables into the prior snapshot: swept tables are
    // reconciled to their current S3 version set; tables absent from this pass (failed to sweep)
    // keep their prior entry. No authored fields are written — those live in the overlay.
    let prior = read_registry(registry_path)
        .await
        .context("read prior snapshot")?
        .unwrap_or_default();
    let mut by_id: HashMap<String, TableEntry> =
        prior.into_iter().map(|e| (e.id.clone(), e)).collect();
    for entry in outcome.tables {
        apply_sweep_result(&mut by_id, entry);
    }

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
        failed_tables: outcome.failed_tables,
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
