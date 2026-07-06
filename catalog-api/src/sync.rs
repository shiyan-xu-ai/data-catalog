//! Sync: refresh the derived registry snapshot for the registered tables.
//!
//! The catalog is a curated allowlist, not a mirror of the bucket: only tables an operator has
//! registered (an authored overlay object exists for them, written by `DeclareTable`) are synced.
//! The sync never enumerates the whole root, so unregistered tables on S3 are ignored.
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
//! first sync publishes progress table-by-table and an interrupted sync resumes via
//! carry-forward instead of starting over.
//!
//! On Cloud Run this runs request-scoped (triggered by Cloud Scheduler hitting
//! `/internal/jobs/sync`), not as a background loop — CPU is throttled between requests, so a
//! `tokio`-spawned loop would freeze. `run_sync` is idempotent and safe under Scheduler
//! double-fire/retry: the snapshot is a complete derived state written last-wins, with no lock
//! or leader (only the sync writes it). Authored state (owner/ttl_policy/protected) lives in
//! the overlay and is never touched here.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use catalog_core::{
    parse_table_id, read_registry, write_registry, Namespace, TableEntry, TableVersion,
};
use catalog_store::{deep_for, MetaStore, SyncConfig, TableMeta, VersionDirRef};
use futures::StreamExt;
use object_store::path::Path as ObjPath;
use serde::Serialize;

/// One sync scope: a single registered namespace within a bucket. The `cfg`'s object store is
/// rooted at the bucket, `cfg.root_path` is the namespace prefix path, and `cfg.root_uri` is
/// `s3://<bucket>/<namespace>`. A sync pass runs over a slice of these (one per registered
/// namespace across all buckets), routing each declared table id to the target whose
/// `(region, bucket, namespace)` it parses to.
#[derive(Clone)]
pub struct SyncTarget {
    pub region: String,
    pub bucket: String,
    /// Namespace prefix segments, e.g. `["a", "b"]` for the `a/b` namespace.
    pub namespace: Vec<String>,
    pub cfg: SyncConfig,
}

/// Minimum spacing between intermediate snapshot writes while a sync is still running. The
/// final write always happens regardless.
const INCREMENTAL_WRITE_INTERVAL: Duration = Duration::from_secs(3);

/// Summary of one sync pass, returned to the scheduler endpoint and logged.
#[derive(Debug, Clone, Serialize)]
pub struct SyncReport {
    pub tables_checked: u64,
    pub failed_tables: u64,
    /// Versions actually synced from S3 this pass (new, partial, or deleting-marked).
    pub versions_synced: u64,
    /// Versions copied from the prior snapshot without any S3 traffic (immutable + clean).
    pub versions_carried: u64,
    /// Objects enumerated across all version LISTs this pass.
    pub objects_listed: u64,
    /// Cumulative time spent in version LISTs (across concurrent syncs, so it can exceed
    /// `duration_secs`). With `open_secs`, shows where sync time actually goes.
    pub list_secs: f64,
    /// Cumulative time spent opening Lance datasets + extracting stats.
    pub open_secs: f64,
    pub duration_secs: f64,
}

/// Per-table outcome of the discovery phase: which prior versions carry forward as-is and
/// which version dirs still need a real sync.
struct TablePlan {
    /// Composite id, exactly as declared in the overlay.
    table_id: String,
    /// The table's leaf name (the last id segment) — the on-store directory name.
    name: String,
    /// Index into the `targets` slice this table was routed to; supplies the store/`cfg` for
    /// its version syncs and the region/bucket/namespace for its assembled entry.
    target_idx: usize,
    carried: Vec<TableVersion>,
    work: Vec<VersionDirRef>,
    /// The table's latest listed version id — drives `DeepStats::Latest` gating.
    latest_id: Option<String>,
}

/// Run one sync over the registered tables. See the module docs for the execution shape.
/// A registered table that fails (listing or any of its versions) is isolated: its prior
/// snapshot entry is kept and `failed_tables` incremented. A registered table with no
/// directory on S3 yet syncs cleanly to zero versions (a stub until data lands).
pub async fn run_sync(
    targets: &[SyncTarget],
    registry_path: &str,
    storage_scan_path: &str,
    meta: &MetaStore,
) -> Result<SyncReport> {
    let started = Instant::now();
    let now = chrono::Utc::now();

    // The registered set = the tables that have an authored overlay. Only these are synced.
    // The overlays also carry the `deleting` markers that veto carry-forward below. `list_meta`
    // runs ONCE for the whole pass; every target filters this same map.
    let registered = meta.list_meta().await.context("list registered tables")?;

    let prior: HashMap<String, TableEntry> = read_registry(registry_path)
        .await
        .context("read prior snapshot")?
        .unwrap_or_default()
        .into_iter()
        .map(|e| (e.id.clone(), e))
        .collect();

    // Route each declared id to the one target whose (region, bucket, namespace) it parses to.
    // An id that parses to no configured target is ignored (it can't be synced — no store for
    // it); an id that fails to parse is likewise skipped.
    struct Routed {
        table_id: String,
        name: String,
        target_idx: usize,
        table_path: ObjPath,
    }
    let mut routed: Vec<Routed> = Vec::new();
    for table_id in registered.keys() {
        let Some(parsed) = parse_table_id(table_id) else {
            continue;
        };
        let Some(target_idx) = targets.iter().position(|t| {
            t.region == parsed.region
                && t.bucket == parsed.bucket
                && t.namespace == parsed.namespace
        }) else {
            continue;
        };
        let table_path = targets[target_idx]
            .cfg
            .root_path
            .clone()
            .join(parsed.name.as_str());
        routed.push(Routed {
            table_id: table_id.clone(),
            name: parsed.name,
            target_idx,
            table_path,
        });
    }

    // The synced allowlist for this pass = the ids that routed to a configured target. Prior
    // entries for routed ids seed the running state so tables not yet synced this pass stay
    // visible in intermediate writes; deregistered (or now-unroutable) ids are dropped
    // immediately (the snapshot is an allowlist). Each table's entry is replaced as it completes.
    let routed_ids: HashSet<&str> = routed.iter().map(|r| r.table_id.as_str()).collect();
    let mut state: HashMap<String, TableEntry> = prior
        .iter()
        .filter(|(id, _)| routed_ids.contains(id.as_str()))
        .map(|(id, e)| (id.clone(), e.clone()))
        .collect();

    let mut failed_tables = 0u64;
    let mut versions_carried = 0u64;

    // Discovery concurrency: use the max across targets (they share one queue below anyway).
    let concurrency = targets.iter().map(|t| t.cfg.concurrency).max().unwrap_or(1);

    // ── Discovery: one delimiter LIST per routed table, concurrently across ALL targets.
    // Partition each table's version dirs into carried (immutable + clean in prior, not marked
    // deleting) vs work (new / partial / deleting-marked → re-sync).
    let discovery_items: Vec<(usize, usize, ObjPath)> = routed
        .iter()
        .enumerate()
        .map(|(i, r)| (i, r.target_idx, r.table_path.clone()))
        .collect();
    let discovery: Vec<(usize, Result<Vec<VersionDirRef>>)> =
        futures::stream::iter(discovery_items)
            .map(|(i, target_idx, path)| async move {
                let dirs = catalog_store::list_version_dirs(&targets[target_idx].cfg, &path).await;
                (i, dirs)
            })
            .buffer_unordered(concurrency)
            .collect()
            .await;

    let mut plans: Vec<TablePlan> = Vec::with_capacity(discovery.len());
    for (i, dirs) in discovery {
        let r = &routed[i];
        match dirs {
            Ok(dirs) => {
                let latest_id = dirs.iter().map(|d| d.version_id.clone()).max();
                let (carried, work) = partition_carry_forward(
                    dirs,
                    prior.get(&r.table_id),
                    registered.get(&r.table_id),
                );
                versions_carried += carried.len() as u64;
                plans.push(TablePlan {
                    table_id: r.table_id.clone(),
                    name: r.name.clone(),
                    target_idx: r.target_idx,
                    carried,
                    work,
                    latest_id,
                });
            }
            Err(e) => {
                failed_tables += 1;
                tracing::warn!(
                    table = %r.table_id,
                    error = %e,
                    "sync: listing registered table failed this cycle; keeping prior entry"
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

    let mut versions_synced = 0u64;
    let mut objects_listed = 0u64;
    let mut list_secs = 0f64;
    let mut open_secs = 0f64;
    let mut last_write = Instant::now();

    // Tables with no pending work (all carried / empty) complete immediately.
    let plan_index: HashMap<String, &TablePlan> =
        plans.iter().map(|p| (p.table_id.clone(), p)).collect();
    for plan in &plans {
        if plan.work.is_empty() {
            let entry = assemble_entry(&targets[plan.target_idx], plan, Vec::new(), now);
            state.insert(plan.table_id.clone(), entry);
        }
    }

    // ── ONE global work queue across every target. Each item carries its target index so its
    // version syncs run against the right bucket's store, but ALL items — from every namespace
    // and bucket — interleave in this single `buffer_unordered` stream. A big namespace can't
    // starve small ones, and wall time is ~ total_work / concurrency, not per-namespace serial.
    let work_items: Vec<(String, usize, VersionDirRef, bool)> = plans
        .iter()
        .flat_map(|p| {
            let deep_stats = targets[p.target_idx].cfg.deep_stats;
            p.work.iter().map(move |d| {
                let deep = deep_for(
                    deep_stats,
                    p.latest_id.as_deref() == Some(d.version_id.as_str()),
                );
                (p.table_id.clone(), p.target_idx, d.clone(), deep)
            })
        })
        .collect();

    let mut results = futures::stream::iter(work_items)
        .map(|(table_id, target_idx, dir, deep)| async move {
            let res = catalog_store::sync_version(
                &targets[target_idx].cfg,
                &dir.path,
                dir.timestamp,
                dir.version_id,
                now,
                deep,
            )
            .await;
            (table_id, res)
        })
        .buffer_unordered(concurrency);

    while let Some((table_id, res)) = results.next().await {
        match res {
            Ok(s) => {
                versions_synced += 1;
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
                        "sync: version sync failed; keeping table's prior entry this cycle"
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
                    &targets[plan.target_idx],
                    plan,
                    fresh.remove(&table_id).unwrap_or_default(),
                    now,
                );
                state.insert(table_id.clone(), entry);
                // Publish progress: long first syncs land table-by-table instead of all-or-
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
    // for a version no longer in the snapshot (TTL-deleted then synced out, or gone out of band)
    // is pruned. This clears the tombstones a TTL apply leaves behind once the version is truly
    // gone. Best-effort — a failure here doesn't fail the sync.
    if let Err(e) = reconcile_overlays(meta, &state).await {
        tracing::warn!(error = %e, "overlay reconciliation failed (non-fatal)");
    }

    // Final last-wins whole-snapshot write — no coordination needed for recomputable derived
    // state.
    let merged: Vec<TableEntry> = state.into_values().collect();
    write_registry(registry_path, &merged)
        .await
        .context("write snapshot")?;

    // Bounded growth: prune registry manifest versions older than a day (each sync, plus its
    // incremental writes, adds a handful).
    if let Err(e) = catalog_core::cleanup_registry(registry_path, chrono::Duration::days(1)).await {
        tracing::warn!(error = %e, "registry version cleanup failed (non-fatal)");
    }

    // Storage analysis tail: a bucket-wide breakdown of registered vs. unexplored prefixes,
    // derived from the snapshot just written. Level-triggered — a failure here is logged and
    // does not fail the sync; the next pass just retries from scratch (the scan is a point-in-
    // time snapshot, not a durable history).
    if let Err(e) = storage_tail(targets, &merged, storage_scan_path, now).await {
        tracing::warn!(error = %e, "storage analysis tail failed (non-fatal)");
    }
    if let Err(e) =
        catalog_core::cleanup_storage_stats(storage_scan_path, chrono::Duration::days(1)).await
    {
        tracing::warn!(error = %e, "storage_scan version cleanup failed (non-fatal)");
    }

    let report = SyncReport {
        // Only the tables that routed to a configured target were checked this pass; a declared
        // id for an unconfigured (region, bucket, namespace) can't be synced and isn't counted.
        tables_checked: routed.len() as u64,
        failed_tables,
        versions_synced,
        versions_carried,
        objects_listed,
        list_secs,
        open_secs,
        duration_secs: started.elapsed().as_secs_f64(),
    };
    tracing::info!(
        tables_checked = report.tables_checked,
        failed_tables = report.failed_tables,
        versions_synced = report.versions_synced,
        versions_carried = report.versions_carried,
        objects_listed = report.objects_listed,
        list_secs = report.list_secs,
        open_secs = report.open_secs,
        duration_secs = report.duration_secs,
        "sync complete"
    );
    Ok(report)
}

/// Storage analysis: for each bucket, report its top-level layout as registered-namespace rows
/// (sized by aggregating the just-written registry `entries`) or unexplored, name-only rows —
/// covering top-level prefixes, loose root objects (the `(root)` pseudo-prefix), and, for each
/// nested registered namespace, its ancestor levels' siblings. Every listing here is a delimiter
/// LIST (`list_with_delimiter`); nothing recurses into a prefix's contents, so this never pays
/// for a full object enumeration the way the version sync does.
async fn storage_tail(
    targets: &[SyncTarget],
    entries: &[TableEntry],
    storage_scan_path: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<usize> {
    use std::collections::{BTreeMap, BTreeSet};
    // One store per bucket (targets share Arc'd stores).
    let mut by_bucket: BTreeMap<&str, (&SyncTarget, Vec<&SyncTarget>)> = BTreeMap::new();
    for t in targets {
        by_bucket
            .entry(t.bucket.as_str())
            .or_insert((t, Vec::new()))
            .1
            .push(t);
    }
    let mut stats = Vec::new();
    for (bucket, (any, ns_targets)) in &by_bucket {
        let store = any.cfg.store.as_ref();
        let registered: BTreeSet<String> =
            ns_targets.iter().map(|t| t.namespace.join("/")).collect();
        let mut emitted: BTreeSet<String> = BTreeSet::new();

        // 1. Bucket root: top-level prefixes + loose objects.
        let root = store.list_with_delimiter(None).await?;
        for p in &root.common_prefixes {
            emitted.insert(p.as_ref().to_string());
        }
        if !root.objects.is_empty() {
            emitted.insert("(root)".to_string());
        }
        // 2. Ancestors of nested namespaces: surface each level's siblings.
        for ns in &registered {
            let segs: Vec<&str> = ns.split('/').collect();
            for depth in 1..segs.len() {
                let ancestor = segs[..depth].join("/");
                let listing = store
                    .list_with_delimiter(Some(&ObjPath::from(ancestor.clone())))
                    .await?;
                emitted.insert(ancestor);
                for p in &listing.common_prefixes {
                    emitted.insert(p.as_ref().to_string());
                }
            }
        }
        // 3. Rows: a prefix IS registered only if it exactly matches a registered namespace;
        // ancestors and siblings stay unexplored (name-only).
        for prefix in emitted {
            let is_ns = registered.contains(&prefix);
            let (bytes, objects, table_count) = if is_ns {
                let in_ns: Vec<&TableEntry> = entries
                    .iter()
                    .filter(|e| e.bucket == *bucket && e.namespace.segments().join("/") == prefix)
                    .collect();
                let bytes: u64 = in_ns
                    .iter()
                    .flat_map(|e| &e.versions)
                    .map(|v| v.storage_bytes_total)
                    .sum();
                let objects: u64 = in_ns
                    .iter()
                    .flat_map(|e| &e.versions)
                    .filter_map(|v| v.object_count)
                    .sum();
                (Some(bytes), Some(objects), Some(in_ns.len() as u32))
            } else {
                (None, None, None)
            };
            stats.push(catalog_core::StoragePrefixStat {
                region: any.region.clone(),
                bucket: (*bucket).to_string(),
                prefix,
                registered: is_ns,
                bytes,
                objects,
                table_count,
                scanned_at: now,
            });
        }
    }
    let n = stats.len();
    catalog_core::write_storage_stats(storage_scan_path, &stats).await?;
    Ok(n)
}

/// Split a table's listed version dirs into carried-forward prior versions and dirs that need
/// a real sync. A version carries forward iff it exists in the prior snapshot with a clean
/// (non-partial) classification and is not marked `deleting` in the overlay:
/// - Immutable timestamp dirs mean a clean version's derived stats can never change, so
///   re-deriving them is pure waste.
/// - A `partial` prior version is re-synced so a transient mis-classification self-heals.
/// - A `deleting`-marked version is re-synced because a failed TTL delete may have removed part
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

/// Build a table's snapshot entry from its carried + freshly-synced versions. Identity
/// (`region`/`bucket`/`namespace`) comes from the routed `target`, not from parsing the store
/// root — a single bucket-rooted store now serves many namespaces, so the root path no longer
/// identifies the namespace. `name` is the id's leaf segment; `root_location` is the namespace
/// URI joined with that name.
fn assemble_entry(
    target: &SyncTarget,
    plan: &TablePlan,
    fresh: Vec<TableVersion>,
    now: chrono::DateTime<chrono::Utc>,
) -> TableEntry {
    let mut versions = plan.carried.clone();
    versions.extend(fresh);
    versions.sort_by(|a, b| a.version_id.cmp(&b.version_id));
    let aux_latest = versions.last().map(|v| v.aux.clone()).unwrap_or_default();
    let root_uri = target.cfg.root_uri.trim_end_matches('/');
    TableEntry {
        id: plan.table_id.clone(),
        name: plan.name.clone(),
        region: target.region.clone(),
        bucket: target.bucket.clone(),
        namespace: Namespace::new(target.namespace.clone()),
        root_location: format!("{root_uri}/{}", plan.name),
        owner: None,
        ttl_policy: None,
        last_synced: Some(now),
        versions,
        aux_latest,
    }
}

/// Prune overlay `protected`/`deleting` version ids that no longer exist in the snapshot for
/// their table (TTL-deleted-and-synced, or removed out of band). Leaves owner/ttl_policy alone.
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
