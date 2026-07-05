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

use anyhow::Result;
use arrow_schema::Schema as ArrowSchema;
use catalog_core::{AuxEntry, Namespace, TableEntry, TableVersion, VersionShape};
use chrono::{DateTime, NaiveDateTime, Utc};
use futures::{StreamExt, TryStreamExt};
use lance::index::DatasetIndexExt;
use lance::Dataset;
use object_store::path::Path as ObjPath;
use object_store::ObjectMeta;

use crate::config::SweepConfig;
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

    let mut versions: Vec<TableVersion> = futures::stream::iter(dirs)
        .map(|d| async move { sweep_version(cfg, &d.path, d.timestamp, d.version_id, now).await })
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
}

async fn try_open_dataset(cfg: &SweepConfig, path: &ObjPath) -> Option<OpenedDataset> {
    let uri = cfg.uri_for(path);
    let dataset = Dataset::open(&uri).await.ok()?;
    let row_count = dataset.count_rows(None).await.ok().map(|n| n as u64);
    let num_fragments = Some(dataset.count_fragments() as u64);
    let schema_json = schema_to_json(dataset.schema()).ok();
    let num_indices = dataset
        .load_indices()
        .await
        .ok()
        .map(|idx| idx.len() as u64);
    Some(OpenedDataset {
        row_count,
        num_fragments,
        schema_json,
        num_indices,
    })
}

/// Build an aux entry for `path` from the pre-fetched object list: real recursive size plus
/// detected format (and fingerprint for mixed/unknown). Pure — no IO.
fn aux_entry_for(
    cfg: &SweepConfig,
    objects: &[ObjectMeta],
    name: &str,
    path: &ObjPath,
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
    }
}

/// Sweep one timestamp-path version dir: classify its shape, split storage bytes into
/// lance-core/sidecar/segments/other-aux, and collect aux entries.
///
/// All of the above comes from ONE recursive LIST of the version dir; the only additional IO
/// is the Lance dataset open for row/schema/index stats.
pub async fn sweep_version(
    cfg: &SweepConfig,
    ts_path: &ObjPath,
    timestamp: DateTime<Utc>,
    version_id: String,
    swept_at: DateTime<Utc>,
) -> Result<TableVersion> {
    // The single LIST: every object under this version, with key/size/etag.
    let objects: Vec<ObjectMeta> = cfg.store.list(Some(ts_path)).try_collect().await?;
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
    let shape;
    let partial;

    if let Some((_main_name, main_path, main_listing)) = &main_dir {
        let opened = try_open_dataset(cfg, main_path).await;

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
                aux.push(aux_entry_for(cfg, &objects, name, &child));
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
            ));
            for name in &inside_sidecar_names {
                let child = main_path.clone().join(name.as_str());
                aux.push(aux_entry_for(cfg, &objects, name, &child));
            }
        } else {
            for name in &inside_sidecar_names {
                let child = main_path.clone().join(name.as_str());
                sidecar_bytes += bytes_under(&objects, &child);
                aux.push(aux_entry_for(cfg, &objects, name, &child));
            }
        }

        match opened {
            Some(o) => {
                row_count = o.row_count;
                num_fragments = o.num_fragments;
                schema_json = o.schema_json;
                num_indices = o.num_indices;
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
        aux.push(aux_entry_for(cfg, &objects, name, path));
    }

    // Segments + other known top-level aux are independent of shape classification.
    if let Some(segments_path) = &top_segments {
        segments_bytes += bytes_under(&objects, segments_path);
        aux.push(aux_entry_for(
            cfg,
            &objects,
            TOP_LEVEL_SEGMENTS_NAME,
            segments_path,
        ));
    }
    for (name, path) in &top_known_aux {
        other_aux_bytes += bytes_under(&objects, path);
        aux.push(aux_entry_for(cfg, &objects, name, path));
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
            ));
        }
    }

    let storage_bytes_total = lance_core_bytes + sidecar_bytes + segments_bytes + other_aux_bytes;

    Ok(TableVersion {
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
        aux,
        swept_at,
    })
}
