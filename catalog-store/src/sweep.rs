//! S3 sweep: discovers tables and timestamp-path versions, classifies each version's
//! shape, splits storage bytes into lance-core/sidecar/segments/other-aux, and collects
//! aux entries — for both the pre- and post-2026-06-26-cutoff sidecar layouts (see
//! findings.md).
//!
//! ## Request shape (performance)
//!
//! A version is swept with **one recursive LIST** of its timestamp directory: every object's
//! key + size + etag lands in memory, and shape classification, byte splits, aux entries,
//! format detection, and fingerprints are all derived from that single object list (see
//! `format.rs`). The only other IO per version is opening the main Lance dataset for
//! row-count/schema/fragment/index stats. Versions sweep concurrently
//! (`SweepConfig::concurrency`), so a table's wall time is ~`versions / concurrency`, not a
//! serial walk of 10+ requests per version.

use std::time::Instant;

use anyhow::Result;
use arrow_schema::Schema as ArrowSchema;
use catalog_core::{AuxEntry, Namespace, TableEntry, TableVersion, VersionShape};
use chrono::{DateTime, NaiveDateTime, Utc};
use futures::{StreamExt, TryStreamExt};
use lance::index::DatasetIndexExt;
use lance::Dataset;
use object_store::path::Path as ObjPath;
use object_store::ObjectMeta;

use crate::config::{DeepStats, SweepConfig};
use crate::format::{bytes_under, children, detect_format, is_lance_shaped, DirListing};

/// Sidecar directory names that, pre-cutoff, live *inside* the main lance dir instead of
/// in a top-level `dataset.sidecar/` (findings.md "Cutoff 2026-06-26 + sidecar placement").
const SIDECAR_INSIDE_LANCE_NAMES: &[&str] = &[
    "_FragmentMetadata",
    "master_indices",
    "lance_tags",
    "lance_tags_intermediate",
    "_asset_replication_segments",
    "_asset_replication_results",
    "curated_indices",
];

/// Lance-core subdirs, wherever the main lance dir is.
const LANCE_CORE_NAMES: &[&str] = &["_versions", "_transactions", "_indices", "data"];

const TOP_LEVEL_SIDECAR_NAME: &str = "dataset.sidecar";
const TOP_LEVEL_SEGMENTS_NAME: &str = "segments";

/// Top-level aux dir names observed in findings.md that are never candidates for "the
/// main lance dir" (arbitrary formats: csv, nested parquet, nested lance).
const KNOWN_TOP_AUX_NAMES: &[&str] = &[
    "curated_csv",
    "row_counts",
    "entity_asset_replication_result",
    "scenario_dataset_etl",
    "single_segment.lance",
    "index_datasets",
    "tag_datasets",
];

/// Parse a timestamp-path directory name in either `YYYY-MM-DD_HH-MM-SS` or
/// `YYYY-MM-DD-HH-MM-SS` form. Returns `None` (skip, don't error) if it matches neither.
fn parse_timestamp_dirname(name: &str) -> Option<DateTime<Utc>> {
    for fmt in ["%Y-%m-%d_%H-%M-%S", "%Y-%m-%d-%H-%M-%S"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(name, fmt) {
            return Some(DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc));
        }
    }
    None
}

fn schema_to_json(schema: &lance::datatypes::Schema) -> Result<String> {
    let arrow_schema: ArrowSchema = schema.into();
    let fields: Vec<serde_json::Value> = arrow_schema
        .fields()
        .iter()
        .map(|f| {
            serde_json::json!({
                "name": f.name(),
                "data_type": format!("{:?}", f.data_type()),
                "nullable": f.is_nullable(),
            })
        })
        .collect();
    Ok(serde_json::to_string(
        &serde_json::json!({ "fields": fields }),
    )?)
}

/// Result of one full sweep pass: the tables that swept cleanly, plus a count of table dirs
/// that failed and were skipped this cycle (per-table failures are isolated so one bad table
/// never aborts the whole cycle).
pub struct SweepOutcome {
    pub tables: Vec<TableEntry>,
    pub failed_tables: u64,
}

/// Sweep the whole configured root: one `TableEntry` per top-level table dir.
///
/// The top-level LIST failing aborts the pass (we can't discover tables). But a single table
/// failing to sweep is isolated: it is logged and skipped (counted in `failed_tables`) so the
/// rest of the catalog still refreshes, rather than one malformed or transiently-unreadable
/// table dir aborting the entire cycle and stalling every other table's freshness.
pub async fn sweep_root(cfg: &SweepConfig) -> Result<SweepOutcome> {
    let listing = cfg.store.list_with_delimiter(Some(&cfg.root_path)).await?;
    let mut tables = Vec::with_capacity(listing.common_prefixes.len());
    let mut failed_tables = 0u64;
    for table_path in listing.common_prefixes {
        let Some(table_name) = table_path.filename().map(|n| n.to_string()) else {
            continue;
        };
        match sweep_table(cfg, &table_name, &table_path).await {
            Ok(entry) => tables.push(entry),
            Err(e) => {
                failed_tables += 1;
                tracing::warn!(
                    table = %table_name,
                    error = %e,
                    "sweep: skipping table that failed to sweep this cycle"
                );
            }
        }
    }
    Ok(SweepOutcome {
        tables,
        failed_tables,
    })
}

/// Whether this version gets the deep (extra-IO) Lance stats under `mode`.
pub fn deep_for(mode: DeepStats, is_latest: bool) -> bool {
    match mode {
        DeepStats::All => true,
        DeepStats::Latest => is_latest,
        DeepStats::None => false,
    }
}

/// One timestamp-path version directory discovered under a table dir: the parsed identity
/// plus the object-store path to sweep. Produced by [`list_version_dirs`]; consumed by
/// [`sweep_version`] — split so callers (e.g. the API's global sweep queue) can decide which
/// versions actually need sweeping (carry-forward) before paying for any per-version IO.
#[derive(Debug, Clone)]
pub struct VersionDirRef {
    pub version_id: String,
    pub timestamp: DateTime<Utc>,
    pub path: ObjPath,
}

/// List a table dir's timestamp-path version directories (one cheap delimiter LIST).
/// Non-timestamp subdirs are skipped, matching discovery's tolerance for stray dirs.
pub async fn list_version_dirs(
    cfg: &SweepConfig,
    table_path: &ObjPath,
) -> Result<Vec<VersionDirRef>> {
    let listing = cfg.store.list_with_delimiter(Some(table_path)).await?;
    let mut dirs = Vec::with_capacity(listing.common_prefixes.len());
    for ts_path in listing.common_prefixes {
        let Some(dirname) = ts_path.filename() else {
            continue;
        };
        let Some(timestamp) = parse_timestamp_dirname(dirname) else {
            continue;
        };
        dirs.push(VersionDirRef {
            version_id: timestamp.to_rfc3339(),
            timestamp,
            path: ts_path,
        });
    }
    Ok(dirs)
}

/// Sweep one table dir: one `TableVersion` per timestamp-path subdir, swept concurrently
/// (`cfg.concurrency` versions in flight). Any version failing fails the table — the caller
/// isolates per-table failures.
pub async fn sweep_table(
    cfg: &SweepConfig,
    table_name: &str,
    table_path: &ObjPath,
) -> Result<TableEntry> {
    let dirs = list_version_dirs(cfg, table_path).await?;
    let now = Utc::now();
    let latest_id = dirs.iter().map(|d| d.version_id.clone()).max();

    let mut versions: Vec<TableVersion> = futures::stream::iter(dirs)
        .map(|d| {
            let deep = deep_for(cfg.deep_stats, latest_id.as_deref() == Some(&d.version_id));
            async move {
                sweep_version(cfg, &d.path, d.timestamp, d.version_id, now, deep)
                    .await
                    .map(|s| s.version)
            }
        })
        .buffer_unordered(cfg.concurrency)
        .try_collect()
        .await?;
    versions.sort_by(|a, b| a.version_id.cmp(&b.version_id));

    let aux_latest = versions.last().map(|v| v.aux.clone()).unwrap_or_default();
    let namespace = Namespace::new(cfg.root_path.parts().map(|p| p.as_ref().to_string()));

    Ok(TableEntry {
        id: table_name.to_string(),
        name: table_name.to_string(),
        namespace,
        root_location: cfg.uri_for(table_path),
        owner: None,
        ttl_policy: None,
        last_swept: Some(now),
        versions,
        aux_latest,
    })
}

/// Everything extracted from opening the main lance dataset, when it is openable.
struct OpenedDataset {
    row_count: Option<u64>,
    num_fragments: Option<u64>,
    schema_json: Option<String>,
    num_indices: Option<u64>,
    lance_version: Option<u64>,
    writer_version: Option<String>,
}

/// Open the main lance dataset and extract stats, preferring the already-fetched manifest over
/// additional IO:
/// - `row_count` comes from the manifest's per-fragment row counts (a pure in-memory sum) when
///   every fragment's count is known; `count_rows()` — which may read deletion files — runs only
///   as a fallback, and only when `deep` is set.
/// - `num_fragments`, schema, the lance manifest version, and the writer version are free
///   manifest reads.
/// - `load_indices` (extra index-metadata IO) runs only when `deep` is set.
///
/// The open itself always runs regardless of `deep`, so openability — and therefore
/// shape/`partial` classification — is identical in every mode.
async fn try_open_dataset(cfg: &SweepConfig, path: &ObjPath, deep: bool) -> Option<OpenedDataset> {
    let uri = cfg.uri_for(path);
    let dataset = Dataset::open(&uri).await.ok()?;
    let manifest = dataset.manifest();

    let all_fragment_rows_known = manifest.fragments.iter().all(|f| f.num_rows().is_some());
    let row_count = if all_fragment_rows_known {
        Some(manifest.summary().total_rows)
    } else if deep {
        dataset.count_rows(None).await.ok().map(|n| n as u64)
    } else {
        None
    };

    let num_fragments = Some(dataset.count_fragments() as u64);
    let schema_json = schema_to_json(dataset.schema()).ok();
    let lance_version = Some(manifest.version);
    let writer_version = manifest
        .writer_version
        .as_ref()
        .map(|w| format!("{}/{}", w.library, w.version));

    let num_indices = if deep {
        dataset
            .load_indices()
            .await
            .ok()
            .map(|idx| idx.len() as u64)
    } else {
        None
    };

    Some(OpenedDataset {
        row_count,
        num_fragments,
        schema_json,
        num_indices,
        lance_version,
        writer_version,
    })
}

/// Placement taxonomy values for [`AuxEntry::category`].
const CATEGORY_SIDECAR: &str = "sidecar";
const CATEGORY_NESTED_SIDECAR: &str = "nested_sidecar";

/// Build an aux entry for `path` from the pre-fetched object list: real recursive size plus
/// detected format (and fingerprint for mixed/unknown). Pure — no IO.
fn aux_entry_for(
    cfg: &SweepConfig,
    objects: &[ObjectMeta],
    name: &str,
    path: &ObjPath,
    category: &str,
) -> AuxEntry {
    let storage_bytes = bytes_under(objects, path);
    let (format, fingerprint) = detect_format(objects, path);
    AuxEntry {
        name: name.to_string(),
        path: cfg.uri_for(path),
        format,
        role: name.to_string(),
        storage_bytes,
        fingerprint,
        category: Some(category.to_string()),
        dataset_path: None,
        row_count: None,
        schema_json: None,
        lance_version: None,
        writer_version: None,
    }
}

/// Every lance dataset root strictly under `base`, found purely from the pre-fetched object
/// list: any directory with a `_versions/` child is a dataset root, at ANY depth — which is how
/// nested sidecar bundles (`dataset.lance/tag_datasets/<n>.lance/.../segment_tags.lance`) are
/// surfaced without a single extra request. Sorted, deduplicated.
fn lance_dataset_roots(objects: &[ObjectMeta], base: &ObjPath) -> Vec<ObjPath> {
    let base_str = format!("{}/", base.as_ref().trim_end_matches('/'));
    let mut roots = std::collections::BTreeSet::new();
    for meta in objects {
        let loc = meta.location.as_ref();
        if !loc.starts_with(&base_str) {
            continue;
        }
        if let Some(idx) = loc.find("/_versions/") {
            let root = &loc[..idx];
            if root.len() > base_str.len() {
                roots.insert(root.to_string());
            }
        }
    }
    roots.into_iter().map(ObjPath::from).collect()
}

/// Open a lance-format aux dataset and fill the entry's manifest-derived stats. Aux stats are
/// intentionally cheaper than the main dataset's: manifest-only row counts (no `count_rows`
/// fallback) and no index loading.
async fn enrich_lance_aux(cfg: &SweepConfig, entry: &mut AuxEntry, root: &ObjPath) {
    let uri = cfg.uri_for(root);
    let Ok(dataset) = Dataset::open(&uri).await else {
        return;
    };
    let manifest = dataset.manifest();
    if manifest.fragments.iter().all(|f| f.num_rows().is_some()) {
        entry.row_count = Some(manifest.summary().total_rows);
    }
    entry.schema_json = schema_to_json(dataset.schema()).ok();
    entry.lance_version = Some(manifest.version);
    entry.writer_version = manifest
        .writer_version
        .as_ref()
        .map(|w| format!("{}/{}", w.library, w.version));
}

/// One swept version plus where its wall time went, so a sweep can report the LIST-vs-Lance
/// split instead of leaving slow tables a mystery.
pub struct SweptVersion {
    pub version: TableVersion,
    /// Objects enumerated by the version's recursive LIST.
    pub objects: u64,
    /// Time spent in the recursive LIST (paginated; ∝ object count).
    pub list_ms: u64,
    /// Time spent opening the Lance dataset + extracting stats (∝ manifest/index complexity).
    pub open_ms: u64,
}

/// Sweep one timestamp-path version dir: classify its shape, split storage bytes into
/// lance-core/sidecar/segments/other-aux, and collect aux entries.
///
/// All of the above comes from ONE recursive LIST of the version dir; the only additional IO
/// is the Lance dataset open for row/schema/index stats (see [`try_open_dataset`] for what
/// `deep` gates).
pub async fn sweep_version(
    cfg: &SweepConfig,
    ts_path: &ObjPath,
    timestamp: DateTime<Utc>,
    version_id: String,
    swept_at: DateTime<Utc>,
    deep: bool,
) -> Result<SweptVersion> {
    // The single LIST: every object under this version, with key/size/etag.
    let list_started = Instant::now();
    let objects: Vec<ObjectMeta> = cfg.store.list(Some(ts_path)).try_collect().await?;
    let list_ms = list_started.elapsed().as_millis() as u64;
    let mut open_ms = 0u64;
    let top = children(&objects, ts_path);

    let mut top_sidecar: Option<ObjPath> = None;
    let mut top_segments: Option<ObjPath> = None;
    let mut top_known_aux: Vec<(String, ObjPath)> = Vec::new();
    let mut candidates: Vec<(String, ObjPath)> = Vec::new();

    for name in &top.subdirs {
        let child = ts_path.clone().join(name.as_str());
        if name == TOP_LEVEL_SIDECAR_NAME {
            top_sidecar = Some(child);
        } else if name == TOP_LEVEL_SEGMENTS_NAME {
            top_segments = Some(child);
        } else if KNOWN_TOP_AUX_NAMES.contains(&name.as_str()) {
            top_known_aux.push((name.clone(), child));
        } else {
            candidates.push((name.clone(), child));
        }
    }

    // Find the main lance dir among the candidates: detected purely by presence of
    // `_versions`+`_transactions` at its own root, never by name (findings.md "Main lance dir
    // detection"). The first lance-shaped candidate wins; every OTHER candidate -- whether it
    // appears before or after the main dir in the listing -- is collected into `leftovers` and
    // folded into `other_aux_bytes` + `aux` below, so no top-level dir's bytes are ever silently
    // dropped from the version's accounting.
    let mut main_dir: Option<(String, ObjPath, DirListing)> = None;
    let mut leftovers: Vec<(String, ObjPath)> = Vec::new();
    for (name, path) in &candidates {
        if main_dir.is_none() {
            let child_listing = children(&objects, path);
            if is_lance_shaped(&child_listing) {
                main_dir = Some((name.clone(), path.clone(), child_listing));
                continue;
            }
        }
        // Not the main lance dir (either not lance-shaped, or a main dir was already found):
        // account for it as an aux/other dir. The first leftover, when there is NO main dir,
        // also drives the `LanceOnlyPartial` shape classification below (findings.md's
        // `dataset.lance/` w/ only `index_datasets/`+`tag_datasets/` case).
        leftovers.push((name.clone(), path.clone()));
    }

    let mut aux: Vec<AuxEntry> = Vec::new();
    let mut lance_core_bytes = 0u64;
    let mut sidecar_bytes = 0u64;
    let mut segments_bytes = 0u64;
    let mut other_aux_bytes = 0u64;
    let mut row_count = None;
    let mut num_fragments = None;
    let mut schema_json = None;
    let mut num_indices = None;
    let mut lance_version = None;
    let mut writer_version = None;
    let shape;
    let partial;

    if let Some((_main_name, main_path, main_listing)) = &main_dir {
        let open_started = Instant::now();
        let opened = try_open_dataset(cfg, main_path, deep).await;
        open_ms = open_started.elapsed().as_millis() as u64;

        for core_name in LANCE_CORE_NAMES {
            if main_listing.subdirs.iter().any(|d| d == core_name) {
                lance_core_bytes += bytes_under(&objects, &main_path.clone().join(*core_name));
            }
        }

        let inside_sidecar_names: Vec<&String> = main_listing
            .subdirs
            .iter()
            .filter(|d| SIDECAR_INSIDE_LANCE_NAMES.contains(&d.as_str()))
            .collect();

        // Any child of the main dir that is neither lance-core nor a known sidecar name
        // (unexpected, but keep it accounted for rather than silently dropping bytes).
        for name in &main_listing.subdirs {
            if !LANCE_CORE_NAMES.contains(&name.as_str())
                && !SIDECAR_INSIDE_LANCE_NAMES.contains(&name.as_str())
            {
                let child = main_path.clone().join(name.as_str());
                other_aux_bytes += bytes_under(&objects, &child);
                aux.push(aux_entry_for(
                    cfg,
                    &objects,
                    name,
                    &child,
                    CATEGORY_NESTED_SIDECAR,
                ));
            }
        }

        if let Some(sidecar_path) = &top_sidecar {
            // TRANSITION DEDUP (findings.md): when sidecar dirs exist BOTH inside the
            // main lance dir AND at top-level `dataset.sidecar/`, prefer the top-level
            // dir as the byte-accounting source of truth (it's the one the writer
            // considers canonical post-dual-write), but still emit an `AuxEntry` for
            // each inside-lance sidecar dir below so nothing is invisible — those
            // entries' own `storage_bytes` are real per-dir sizes, they're just excluded
            // from the `sidecar_bytes` aggregate to avoid double-counting.
            sidecar_bytes += bytes_under(&objects, sidecar_path);
            aux.push(aux_entry_for(
                cfg,
                &objects,
                TOP_LEVEL_SIDECAR_NAME,
                sidecar_path,
                CATEGORY_SIDECAR,
            ));
            for name in &inside_sidecar_names {
                let child = main_path.clone().join(name.as_str());
                aux.push(aux_entry_for(
                    cfg,
                    &objects,
                    name,
                    &child,
                    CATEGORY_NESTED_SIDECAR,
                ));
            }
        } else {
            for name in &inside_sidecar_names {
                let child = main_path.clone().join(name.as_str());
                sidecar_bytes += bytes_under(&objects, &child);
                aux.push(aux_entry_for(
                    cfg,
                    &objects,
                    name,
                    &child,
                    CATEGORY_NESTED_SIDECAR,
                ));
            }
        }

        match opened {
            Some(o) => {
                row_count = o.row_count;
                num_fragments = o.num_fragments;
                schema_json = o.schema_json;
                num_indices = o.num_indices;
                lance_version = o.lance_version;
                writer_version = o.writer_version;
                shape = if top_segments.is_some() {
                    VersionShape::Full
                } else {
                    VersionShape::LanceOnly
                };
                partial = false;
            }
            None => {
                // Looked lance-shaped (had _versions/_transactions) but failed to open
                // (e.g. corrupt manifest). Degrade gracefully rather than panic/error out
                // the whole sweep: record it as partial with no schema/row data.
                shape = VersionShape::LanceOnlyPartial;
                partial = true;
            }
        }
    } else if !leftovers.is_empty() {
        // No openable main lance dir, but at least one unrecognized top-level dir is present
        // (e.g. a `dataset.lance/` holding only nested lances). Classify partial; the dir(s)
        // are byte-accounted in the unconditional leftover loop below.
        shape = VersionShape::LanceOnlyPartial;
        partial = true;
    } else if top_segments.is_some() || !top_known_aux.is_empty() || top_sidecar.is_some() {
        shape = VersionShape::SegOnly;
        partial = true;
    } else {
        shape = VersionShape::Empty;
        partial = true;
    }

    // Account for EVERY leftover top-level candidate dir (siblings of the main lance dir that
    // are neither lance-core, sidecar, segments, nor a known aux name), regardless of shape, so
    // their bytes land in `other_aux_bytes` and they surface as aux entries instead of being
    // silently dropped from the version's size accounting.
    for (name, path) in &leftovers {
        other_aux_bytes += bytes_under(&objects, path);
        aux.push(aux_entry_for(cfg, &objects, name, path, CATEGORY_SIDECAR));
    }

    // Segments + other known top-level aux are independent of shape classification.
    if let Some(segments_path) = &top_segments {
        segments_bytes += bytes_under(&objects, segments_path);
        aux.push(aux_entry_for(
            cfg,
            &objects,
            TOP_LEVEL_SEGMENTS_NAME,
            segments_path,
            CATEGORY_SIDECAR,
        ));
    }
    for (name, path) in &top_known_aux {
        other_aux_bytes += bytes_under(&objects, path);
        aux.push(aux_entry_for(cfg, &objects, name, path, CATEGORY_SIDECAR));
    }
    // A top-level `dataset.sidecar/` with no valid main lance dir at all (main_dir is
    // None) still counts toward storage/aux even though there's nothing to dedup against.
    if main_dir.is_none() {
        if let Some(sidecar_path) = &top_sidecar {
            sidecar_bytes += bytes_under(&objects, sidecar_path);
            aux.push(aux_entry_for(
                cfg,
                &objects,
                TOP_LEVEL_SIDECAR_NAME,
                sidecar_path,
                CATEGORY_SIDECAR,
            ));
        }
    }

    // ── Nested lance datasets. Every dataset root under this version (any depth) is already
    // visible in the object list. An existing aux entry whose path IS a root gets enriched in
    // place; a root deeper than any entry (nested bundles like
    // `dataset.lance/tag_datasets/<n>.lance/.../segment_tags.lance`) becomes its own entry,
    // named by its path relative to the version dir. Byte-split buckets are untouched — a
    // nested entry's bytes are a subset of its top dir's, by design. Category: roots under a
    // main-candidate dir (`dataset.lance`-like) are `nested_sidecar`; the rest are `sidecar`.
    // Opens are deep-gated like the main dataset's expensive stats.
    let enrich_started = Instant::now();
    let ts_prefix = format!("{}/", ts_path.as_ref().trim_end_matches('/'));
    let candidate_names: Vec<&str> = candidates.iter().map(|(n, _)| n.as_str()).collect();
    let main_root = main_dir.as_ref().map(|(_, p, _)| p.as_ref().to_string());
    for root in lance_dataset_roots(&objects, ts_path) {
        let root_str = root.as_ref().to_string();
        if main_root.as_deref() == Some(root_str.as_str()) {
            continue; // the main dataset, not aux
        }
        let rel = root_str
            .strip_prefix(&ts_prefix)
            .unwrap_or(root_str.as_str())
            .to_string();
        let top_segment = rel.split('/').next().unwrap_or_default();
        let category = if candidate_names.contains(&top_segment) {
            CATEGORY_NESTED_SIDECAR
        } else {
            CATEGORY_SIDECAR
        };
        let uri = cfg.uri_for(&root);
        let entry = match aux.iter_mut().find(|a| a.path == uri) {
            Some(existing) => {
                existing.dataset_path = Some(uri.clone());
                existing
            }
            None => {
                let mut e = aux_entry_for(cfg, &objects, &rel, &root, category);
                e.dataset_path = Some(uri.clone());
                aux.push(e);
                aux.last_mut().expect("just pushed")
            }
        };
        if deep {
            enrich_lance_aux(cfg, entry, &root).await;
        }
    }
    open_ms += enrich_started.elapsed().as_millis() as u64;

    let storage_bytes_total = lance_core_bytes + sidecar_bytes + segments_bytes + other_aux_bytes;

    let objects_count = objects.len() as u64;
    tracing::debug!(
        version = %version_id,
        path = %ts_path,
        objects = objects_count,
        list_ms,
        open_ms,
        deep,
        "swept version"
    );

    Ok(SweptVersion {
        version: TableVersion {
            version_id,
            timestamp,
            snapshot_path: cfg.uri_for(ts_path),
            shape,
            partial,
            protected: false,
            storage_bytes_total,
            lance_core_bytes,
            sidecar_bytes,
            segments_bytes,
            other_aux_bytes,
            row_count,
            num_fragments,
            schema_json,
            num_indices,
            lance_version,
            writer_version,
            aux,
            swept_at,
        },
        objects: objects_count,
        list_ms,
        open_ms,
    })
}
