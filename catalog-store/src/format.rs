//! Format detection and byte-accounting helpers shared by the sweep.
//!
//! The classification helpers here are pure functions over a pre-fetched object list: the sweep
//! fetches every object under a version directory with ONE recursive LIST, then derives listings,
//! byte totals, formats, and fingerprints in memory instead of issuing per-directory S3 requests
//! (the old shape of this module). `recursive_bytes` remains as the one direct-IO helper for
//! callers that only need a size (e.g. TTL tests).

use std::collections::hash_map::DefaultHasher;
use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};

use anyhow::Result;
use futures::TryStreamExt;
use object_store::path::Path as ObjPath;
use object_store::{ObjectMeta, ObjectStore};

use catalog_core::AuxFormat;

/// One-level directory listing: file names and immediate subdirectory names (basenames,
/// not full paths).
pub struct DirListing {
    pub files: Vec<String>,
    pub subdirs: Vec<String>,
}

/// The path of `meta` relative to `dir`, or `None` if it is not strictly under `dir`.
fn rel_under<'a>(meta: &'a ObjectMeta, dir: &str) -> Option<&'a str> {
    let loc = meta.location.as_ref();
    loc.strip_prefix(dir)
        .and_then(|rest| rest.strip_prefix('/'))
        .filter(|rest| !rest.is_empty())
}

/// Compute the immediate children of `dir` from a pre-fetched object list — the in-memory
/// equivalent of a one-level (delimiter) LIST. Subdir names come back in lexicographic order,
/// matching S3's listing order, which the main-lance-dir detection depends on.
pub fn children(objects: &[ObjectMeta], dir: &ObjPath) -> DirListing {
    let dir = dir.as_ref().trim_end_matches('/');
    let mut files = BTreeSet::new();
    let mut subdirs = BTreeSet::new();
    for meta in objects {
        let Some(rest) = rel_under(meta, dir) else {
            continue;
        };
        match rest.split_once('/') {
            Some((first, _)) => {
                subdirs.insert(first.to_string());
            }
            None => {
                files.insert(rest.to_string());
            }
        }
    }
    DirListing {
        files: files.into_iter().collect(),
        subdirs: subdirs.into_iter().collect(),
    }
}

/// Sum the sizes of all objects strictly under `dir` from a pre-fetched object list — the
/// in-memory equivalent of the old per-directory recursive LIST.
pub fn bytes_under(objects: &[ObjectMeta], dir: &ObjPath) -> u64 {
    let dir = dir.as_ref().trim_end_matches('/');
    objects
        .iter()
        .filter(|m| rel_under(m, dir).is_some())
        .map(|m| m.size)
        .sum()
}

/// Sum the sizes of all objects recursively under `prefix`. No GETs — LIST metadata only.
pub async fn recursive_bytes(store: &dyn ObjectStore, prefix: &ObjPath) -> Result<u64> {
    let mut stream = store.list(Some(prefix));
    let mut total = 0u64;
    while let Some(meta) = stream.try_next().await? {
        total += meta.size;
    }
    Ok(total)
}

/// Cheap, non-cryptographic fingerprint over the sorted (key, etag, size) tuples of every
/// object under `dir`. Used for `Mixed`/`Unknown` aux dirs where we don't otherwise inspect
/// content (design doc §3.3). Same tuples — full object key, etag, size — as the old
/// LIST-per-call implementation, so values are unchanged for unchanged content.
pub fn fingerprint(objects: &[ObjectMeta], dir: &ObjPath) -> String {
    let dir_str = dir.as_ref().trim_end_matches('/');
    let mut tuples: Vec<(String, String, u64)> = objects
        .iter()
        .filter(|m| rel_under(m, dir_str).is_some())
        .map(|m| {
            (
                m.location.as_ref().to_string(),
                m.e_tag.clone().unwrap_or_default(),
                m.size,
            )
        })
        .collect();
    tuples.sort();
    let mut hasher = DefaultHasher::new();
    tuples.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Does this one-level directory listing look like a lance dataset root (has both
/// `_versions` and `_transactions` immediate subdirs)?
pub fn is_lance_shaped(listing: &DirListing) -> bool {
    listing.subdirs.iter().any(|d| d == "_versions")
        && listing.subdirs.iter().any(|d| d == "_transactions")
}

/// Detect the format of an aux directory per findings.md's format-detection rules:
/// parquet if `_SUCCESS` + `part-*.parquet` files are present, lance if it looks like a
/// lance dataset root, csv if it contains `*.csv` files, else mixed/unknown (with a
/// fingerprint since we don't know how to summarize the content).
pub fn detect_format(objects: &[ObjectMeta], dir: &ObjPath) -> (AuxFormat, Option<String>) {
    let listing = children(objects, dir);

    let has_success = listing.files.iter().any(|f| f == "_SUCCESS");
    let has_parquet_part = listing
        .files
        .iter()
        .any(|f| f.starts_with("part-") && f.ends_with(".parquet"));
    if has_success && has_parquet_part {
        return (AuxFormat::Parquet, None);
    }

    if is_lance_shaped(&listing) {
        return (AuxFormat::Lance, None);
    }

    if listing.files.iter().any(|f| f.ends_with(".csv")) {
        return (AuxFormat::Csv, None);
    }

    let fp = fingerprint(objects, dir);
    if listing.files.is_empty() && listing.subdirs.is_empty() {
        (AuxFormat::Unknown, Some(fp))
    } else {
        (AuxFormat::Mixed, Some(fp))
    }
}
