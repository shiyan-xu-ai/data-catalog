//! Sweep: refresh the derived registry snapshot for the registered tables.
//!
//! The catalog is a curated allowlist, not a mirror of the bucket: only tables an operator has
//! registered (an authored overlay object exists for them, written by `DeclareTable`) are swept.
//! The sweep never enumerates the whole root, so unregistered tables on S3 are ignored.
//!
//! ## Execution shape (performance)
//!
//! The unit of work is the **version**, not the table, so a 500-version table doesn't serialize
//! behind (or starve) small tables: all registered tables' version dirs are discovered first
//! (one cheap delimiter LIST per table, concurrently), versions already in the prior snapshot
//! are **carried forward** without any S3 traffic (they're immutable timestamp dirs — their
//! derived stats can't change), and the remaining versions from ALL tables feed one global
//! `buffer_unordered` queue. Wall time ≈ `new versions / concurrency`.
//!
//! As each table's last version completes, the snapshot is rewritten (throttled) — so a long
//! first sweep publishes progress table-by-table and an interrupted sweep resumes via
//! carry-forward instead of starting over.
//!
//! On Cloud Run this runs request-scoped (triggered by Cloud Scheduler hitting
//! `/internal/jobs/sweep`), not as a background loop — CPU is throttled between requests, so a
//! `tokio`-spawned loop would freeze. `run_sweep` is idempotent and safe under Scheduler
//! double-fire/retry: the snapshot is a complete derived state written last-wins, with no lock
//! or leader (only the sweep writes it). Authored state (owner/ttl_policy/protected) lives in
//! the overlay and is never touched here.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use catalog_core::{read_registry, write_registry, TableEntry, TableVersion};
use catalog_store::{deep_for, MetaStore, SweepConfig, TableMeta, VersionDirRef};
use futures::StreamExt;
use object_store::path::Path as ObjPath;
use serde::Serialize;

/// Minimum spacing between intermediate snapshot writes while a sweep is still running. The
/// final write always happens regardless.
const INCREMENTAL_WRITE_INTERVAL: Duration = Duration::from_secs(3);

/// Summary of one sweep pass, returned to the scheduler endpoint and logged.
#[derive(Debug, Clone, Serialize)]
pub struct SweepReport {
    pub tables_checked: u64,
    pub failed_tables: u64,
    /// Versions actually swept from S3 this pass (new, partial, or deleting-marked).
    pub versions_swept: u64,
    /// Versions copied from the prior snapshot without any S3 traffic (immutable + clean).
    pub versions_carried: u64,
    /// Objects enumerated across all version LISTs this pass.
    pub objects_listed: u64,
    /// Cumulative time spent in version LISTs (across concurrent sweeps, so it can exceed
    /// `duration_secs`). With `open_secs`, shows where sweep time actually goes.
    pub list_secs: f64,
    /// Cumulative time spent opening Lance datasets + extracting stats.
    pub open_secs: f64,
    pub duration_secs: f64,
}

/// Per-table outcome of the discovery phase: which prior versions carry forward as-is and
/// which version dirs still need a real sweep.
struct TablePlan {
    table_id: String,
    table_path: ObjPath,
    carried: Vec<TableVersion>,
    work: Vec<VersionDirRef>,
    /// The table's latest listed version id — drives `DeepStats::Latest` gating.
    latest_id: Option<String>,
}

/// Run one sweep over the registered tables. See the module docs for the execution shape.
/// A registered table that fails (listing or any of its versions) is isolated: its prior
/// snapshot entry is kept and `failed_tables` incremented. A registered table with no
/// directory on S3 yet sweeps cleanly to zero versions (a stub until data lands).
pub async fn run_sweep(
    sweep_cfg: &SweepConfig,
    registry_path: &str,
    meta: &MetaStore,
) -> Result<SweepReport> {
    let started = Instant::now();
    let now = chrono::Utc::now();

    // The registered set = the tables that have an authored overlay. Only these are swept.
    // The overlays also carry the `deleting` markers that veto carry-forward below.
    let registered = meta.list_meta().await.context("list registered tables")?;

    let prior: HashMap<String, TableEntry> = read_registry(registry_path)
        .await
        .context("read prior snapshot")?
        .unwrap_or_default()
        .into_iter()
        .map(|e| (e.id.clone(), e))
        .collect();

    // Running snapshot state. Seeded with prior ∩ registered so tables not yet swept this pass
    // stay visible in intermediate writes; a deregistered table is dropped immediately (the
    // snapshot is an allowlist). Each table's entry is replaced as it completes.
    let mut state: HashMap<String, TableEntry> = prior
        .iter()
        .filter(|(id, _)| registered.contains_key(*id))
        .map(|(id, e)| (id.clone(), e.clone()))
        .collect();

    let mut failed_tables = 0u64;
    let mut versions_carried = 0u64;

    // ── Discovery: one delimiter LIST per registered table, concurrently. Partition each
    // table's version dirs into carried (immutable + clean in prior, not marked deleting)
    // vs work (new / partial / deleting-marked → re-sweep).
    let discovery: Vec<(String, Result<Vec<VersionDirRef>>)> =
        futures::stream::iter(registered.keys().cloned())
            .map(|table_id| async move {
                let path = table_path(sweep_cfg, &table_id);
                let dirs = catalog_store::list_version_dirs(sweep_cfg, &path).await;
                (table_id, dirs)
            })
            .buffer_unordered(sweep_cfg.concurrency)
            .collect()
            .await;

    let mut plans: Vec<TablePlan> = Vec::with_capacity(discovery.len());
    for (table_id, dirs) in discovery {
        match dirs {
            Ok(dirs) => {
                let latest_id = dirs.iter().map(|d| d.version_id.clone()).max();
                let (carried, work) =
                    partition_carry_forward(dirs, prior.get(&table_id), registered.get(&table_id));
                versions_carried += carried.len() as u64;
                plans.push(TablePlan {
                    table_path: table_path(sweep_cfg, &table_id),
                    table_id,
                    carried,
                    work,
                    latest_id,
                });
            }
            Err(e) => {
                failed_tables += 1;
                tracing::warn!(
                    table = %table_id,
                    error = %e,
                    "sweep: listing registered table failed this cycle; keeping prior entry"
                );
                // Isolation: `state` already holds the prior entry (if any); nothing to do.
            }
        }
    }

    // ── Global version queue: every table's pending versions interleave in one
    // buffer_unordered stream, so a 500-version table can't starve small ones and wall time
    // is ~ total_work / concurrency. Results are collected per table; a table is assembled
    // and published as soon as its last version lands.
    let mut remaining: HashMap<String, usize> = HashMap::new();
    let mut fresh: HashMap<String, Vec<TableVersion>> = HashMap::new();
    let mut table_failed: HashSet<String> = HashSet::new();
    for plan in &plans {
        remaining.insert(plan.table_id.clone(), plan.work.len());
        fresh.insert(plan.table_id.clone(), Vec::new());
    }

    let mut versions_swept = 0u64;
    let mut objects_listed = 0u64;
    let mut list_secs = 0f64;
    let mut open_secs = 0f64;
    let mut last_write = Instant::now();

    // Tables with no pending work (all carried / empty) complete immediately.
    let plan_index: HashMap<String, &TablePlan> =
        plans.iter().map(|p| (p.table_id.clone(), p)).collect();
    for plan in &plans {
        if plan.work.is_empty() {
            let entry = assemble_entry(sweep_cfg, plan, Vec::new(), now);
            state.insert(plan.table_id.clone(), entry);
        }
    }

    let work_items: Vec<(String, VersionDirRef, bool)> = plans
        .iter()
        .flat_map(|p| {
            p.work.iter().map(|d| {
                let deep = deep_for(
                    sweep_cfg.deep_stats,
                    p.latest_id.as_deref() == Some(d.version_id.as_str()),
                );
                (p.table_id.clone(), d.clone(), deep)
            })
        })
        .collect();

    let mut results = futures::stream::iter(work_items)
        .map(|(table_id, dir, deep)| async move {
            let res = catalog_store::sweep_version(
                sweep_cfg,
                &dir.path,
                dir.timestamp,
                dir.version_id,
                now,
                deep,
            )
            .await;
            (table_id, res)
        })
        .buffer_unordered(sweep_cfg.concurrency);

    while let Some((table_id, res)) = results.next().await {
        match res {
            Ok(s) => {
                versions_swept += 1;
                objects_listed += s.objects;
                list_secs += s.list_ms as f64 / 1000.0;
                open_secs += s.open_ms as f64 / 1000.0;
                fresh
                    .get_mut(&table_id)
                    .expect("planned table")
                    .push(s.version);
            }
            Err(e) => {
                if table_failed.insert(table_id.clone()) {
                    tracing::warn!(
                        table = %table_id,
                        error = %e,
                        "sweep: version sweep failed; keeping table's prior entry this cycle"
                    );
                }
            }
        }
        let left = remaining.get_mut(&table_id).expect("planned table");
        *left -= 1;
        if *left == 0 {
            if table_failed.contains(&table_id) {
                failed_tables += 1;
                // Isolation: leave the prior entry (already seeded in `state`) untouched.
            } else {
                let plan = plan_index[&table_id];
                let entry = assemble_entry(
                    sweep_cfg,
                    plan,
                    fresh.remove(&table_id).unwrap_or_default(),
                    now,
                );
                state.insert(table_id.clone(), entry);
                // Publish progress: long first sweeps land table-by-table instead of all-or-
                // nothing at the end. Throttled; last-wins writes make intermediates safe.
                if last_write.elapsed() >= INCREMENTAL_WRITE_INTERVAL {
                    let entries: Vec<TableEntry> = state.values().cloned().collect();
                    if let Err(e) = write_registry(registry_path, &entries).await {
                        tracing::warn!(error = %e, "incremental snapshot write failed (non-fatal)");
                    }
                    last_write = Instant::now();
                }
            }
        }
    }

    // Reconcile stale overlay markers to the new snapshot truth: a `protected`/`deleting` entry
    // for a version no longer in the snapshot (TTL-deleted then swept out, or gone out of band)
    // is pruned. This clears the tombstones a TTL apply leaves behind once the version is truly
    // gone. Best-effort — a failure here doesn't fail the sweep.
    if let Err(e) = reconcile_overlays(meta, &state).await {
        tracing::warn!(error = %e, "overlay reconciliation failed (non-fatal)");
    }

    // Final last-wins whole-snapshot write — no coordination needed for recomputable derived
    // state.
    let merged: Vec<TableEntry> = state.into_values().collect();
    write_registry(registry_path, &merged)
        .await
        .context("write snapshot")?;

    // Bounded growth: prune registry manifest versions older than a day (each sweep, plus its
    // incremental writes, adds a handful).
    if let Err(e) = catalog_core::cleanup_registry(registry_path, chrono::Duration::days(1)).await {
        tracing::warn!(error = %e, "registry version cleanup failed (non-fatal)");
    }

    let report = SweepReport {
        tables_checked: registered.len() as u64,
        failed_tables,
        versions_swept,
        versions_carried,
        objects_listed,
        list_secs,
        open_secs,
        duration_secs: started.elapsed().as_secs_f64(),
    };
    tracing::info!(
        tables_checked = report.tables_checked,
        failed_tables = report.failed_tables,
        versions_swept = report.versions_swept,
        versions_carried = report.versions_carried,
        objects_listed = report.objects_listed,
        list_secs = report.list_secs,
        open_secs = report.open_secs,
        duration_secs = report.duration_secs,
        "sweep complete"
    );
    Ok(report)
}

/// Split a table's listed version dirs into carried-forward prior versions and dirs that need
/// a real sweep. A version carries forward iff it exists in the prior snapshot with a clean
/// (non-partial) classification and is not marked `deleting` in the overlay:
/// - Immutable timestamp dirs mean a clean version's derived stats can never change, so
///   re-deriving them is pure waste.
/// - A `partial` prior version is re-swept so a transient mis-classification self-heals.
/// - A `deleting`-marked version is re-swept because a failed TTL delete may have removed part
///   of its prefix — carrying it would freeze pre-delete byte counts.
///
/// Versions in the prior snapshot but absent from the listing are simply dropped (deleted from
/// S3): they appear in neither output.
fn partition_carry_forward(
    dirs: Vec<VersionDirRef>,
    prior: Option<&TableEntry>,
    overlay: Option<&TableMeta>,
) -> (Vec<TableVersion>, Vec<VersionDirRef>) {
    let prior_versions: HashMap<&str, &TableVersion> = prior
        .map(|e| {
            e.versions
                .iter()
                .map(|v| (v.version_id.as_str(), v))
                .collect()
        })
        .unwrap_or_default();
    let deleting: Option<&std::collections::BTreeSet<String>> = overlay.map(|m| &m.deleting);

    let mut carried = Vec::new();
    let mut work = Vec::new();
    for dir in dirs {
        let marked_deleting = deleting.is_some_and(|d| d.contains(&dir.version_id));
        match prior_versions.get(dir.version_id.as_str()) {
            Some(prev) if !prev.partial && !marked_deleting => carried.push((*prev).clone()),
            _ => work.push(dir),
        }
    }
    (carried, work)
}

/// Build a table's snapshot entry from its carried + freshly-swept versions.
fn assemble_entry(
    sweep_cfg: &SweepConfig,
    plan: &TablePlan,
    fresh: Vec<TableVersion>,
    now: chrono::DateTime<chrono::Utc>,
) -> TableEntry {
    let mut versions = plan.carried.clone();
    versions.extend(fresh);
    versions.sort_by(|a, b| a.version_id.cmp(&b.version_id));
    let aux_latest = versions.last().map(|v| v.aux.clone()).unwrap_or_default();
    let namespace =
        catalog_core::Namespace::new(sweep_cfg.root_path.parts().map(|p| p.as_ref().to_string()));
    TableEntry {
        id: plan.table_id.clone(),
        name: plan.table_id.clone(),
        namespace,
        root_location: sweep_cfg.uri_for(&plan.table_path),
        owner: None,
        ttl_policy: None,
        last_swept: Some(now),
        versions,
        aux_latest,
    }
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
