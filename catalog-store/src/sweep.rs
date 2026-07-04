//! S3 sweep: discovers tables and timestamp-path versions, classifies each version's
//! shape, splits storage bytes into lance-core/sidecar/segments/other-aux, and collects
//! aux entries — for both the pre- and post-2026-06-26-cutoff sidecar layouts (see
//! findings.md).

use anyhow::{Context, Result};
use arrow_schema::Schema as ArrowSchema;
use catalog_core::{AuxEntry, Namespace, TableEntry, TableVersion, VersionShape};
use chrono::{DateTime, NaiveDateTime, Utc};
use lance::index::DatasetIndexExt;
use lance::Dataset;
use object_store::path::Path as ObjPath;
use object_store::ObjectStore;

use crate::config::SweepConfig;
use crate::format::{detect_format, is_lance_shaped, list_dir, recursive_bytes, DirListing};

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

/// Sweep the whole configured root: one `TableEntry` per top-level table dir.
pub async fn sweep_root(cfg: &SweepConfig) -> Result<Vec<TableEntry>> {
    let listing = cfg.store.list_with_delimiter(Some(&cfg.root_path)).await?;
    let mut tables = Vec::with_capacity(listing.common_prefixes.len());
    for table_path in listing.common_prefixes {
        let table_name = table_path
            .filename()
            .context("table dir has no name")?
            .to_string();
        tables.push(sweep_table(cfg, &table_name, &table_path).await?);
    }
    Ok(tables)
}

/// Sweep one table dir: one `TableVersion` per timestamp-path subdir.
pub async fn sweep_table(
    cfg: &SweepConfig,
    table_name: &str,
    table_path: &ObjPath,
) -> Result<TableEntry> {
    let listing = cfg.store.list_with_delimiter(Some(table_path)).await?;
    let now = Utc::now();

    let mut versions = Vec::new();
    for ts_path in listing.common_prefixes {
        let Some(dirname) = ts_path.filename() else {
            continue;
        };
        let Some(timestamp) = parse_timestamp_dirname(dirname) else {
            continue;
        };
        let version_id = timestamp.to_rfc3339();
        versions.push(sweep_version(cfg, &ts_path, timestamp, version_id, now).await?);
    }
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

async fn aux_entry_for(cfg: &SweepConfig, name: &str, path: &ObjPath) -> Result<AuxEntry> {
    let storage_bytes = recursive_bytes(cfg.store.as_ref(), path).await?;
    let (format, fingerprint) = detect_format(cfg.store.as_ref(), path).await?;
    Ok(AuxEntry {
        name: name.to_string(),
        path: cfg.uri_for(path),
        format,
        role: name.to_string(),
        storage_bytes,
        fingerprint,
    })
}

/// Sweep one timestamp-path version dir: classify its shape, split storage bytes into
/// lance-core/sidecar/segments/other-aux, and collect aux entries.
pub async fn sweep_version(
    cfg: &SweepConfig,
    ts_path: &ObjPath,
    timestamp: DateTime<Utc>,
    version_id: String,
    swept_at: DateTime<Utc>,
) -> Result<TableVersion> {
    let store: &dyn ObjectStore = cfg.store.as_ref();
    let top = list_dir(store, ts_path).await?;

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

    // Find the main lance dir among the leftover candidates: detected purely by presence
    // of `_versions`+`_transactions` at its own root, never by name (findings.md "Main
    // lance dir detection").
    let mut main_dir: Option<(String, ObjPath, DirListing)> = None;
    let mut partial_candidate: Option<(String, ObjPath)> = None;
    for (name, path) in &candidates {
        let child_listing = list_dir(store, path).await?;
        if is_lance_shaped(&child_listing) {
            main_dir = Some((name.clone(), path.clone(), child_listing));
            break;
        } else if partial_candidate.is_none() {
            // AMBIGUOUS DESIGN CALL: any non-lance-shaped leftover top-level dir (once
            // known sidecar/segments/aux names are excluded) is treated as the
            // "attempted main dataset dir" for LanceOnlyPartial classification, per
            // findings.md's `dataset.lance/` w/ only `index_datasets/`+`tag_datasets/`
            // case. We don't recurse to verify it truly contains *only* nested lances —
            // if it turns out to be some other unrecognized top-level dir, it still gets
            // folded into `other_aux_bytes` below, so nothing is silently dropped; it's
            // just classified as `partial` rather than a clean shape.
            partial_candidate = Some((name.clone(), path.clone()));
        }
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
                lance_core_bytes +=
                    recursive_bytes(store, &main_path.clone().join(*core_name)).await?;
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
                other_aux_bytes += recursive_bytes(store, &child).await?;
                aux.push(aux_entry_for(cfg, name, &child).await?);
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
            sidecar_bytes += recursive_bytes(store, sidecar_path).await?;
            aux.push(aux_entry_for(cfg, TOP_LEVEL_SIDECAR_NAME, sidecar_path).await?);
            for name in &inside_sidecar_names {
                let child = main_path.clone().join(name.as_str());
                aux.push(aux_entry_for(cfg, name, &child).await?);
            }
        } else {
            for name in &inside_sidecar_names {
                let child = main_path.clone().join(name.as_str());
                sidecar_bytes += recursive_bytes(store, &child).await?;
                aux.push(aux_entry_for(cfg, name, &child).await?);
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
    } else if let Some((name, path)) = &partial_candidate {
        shape = VersionShape::LanceOnlyPartial;
        partial = true;
        other_aux_bytes += recursive_bytes(store, path).await?;
        aux.push(aux_entry_for(cfg, name, path).await?);
    } else if top_segments.is_some() || !top_known_aux.is_empty() || top_sidecar.is_some() {
        shape = VersionShape::SegOnly;
        partial = true;
    } else {
        shape = VersionShape::Empty;
        partial = true;
    }

    // Segments + other known top-level aux are independent of shape classification.
    if let Some(segments_path) = &top_segments {
        segments_bytes += recursive_bytes(store, segments_path).await?;
        aux.push(aux_entry_for(cfg, TOP_LEVEL_SEGMENTS_NAME, segments_path).await?);
    }
    for (name, path) in &top_known_aux {
        other_aux_bytes += recursive_bytes(store, path).await?;
        aux.push(aux_entry_for(cfg, name, path).await?);
    }
    // A top-level `dataset.sidecar/` with no valid main lance dir at all (main_dir is
    // None) still counts toward storage/aux even though there's nothing to dedup against.
    if main_dir.is_none() {
        if let Some(sidecar_path) = &top_sidecar {
            sidecar_bytes += recursive_bytes(store, sidecar_path).await?;
            aux.push(aux_entry_for(cfg, TOP_LEVEL_SIDECAR_NAME, sidecar_path).await?);
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
