//! Format detection and byte-accounting helpers shared by the sweep.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use anyhow::Result;
use futures::TryStreamExt;
use object_store::path::Path as ObjPath;
use object_store::ObjectStore;

use catalog_core::AuxFormat;

/// One-level directory listing: file names and immediate subdirectory names (basenames,
/// not full paths).
pub struct DirListing {
    pub files: Vec<String>,
    pub subdirs: Vec<String>,
}

fn basename(path: &ObjPath) -> String {
    path.filename().unwrap_or_default().to_string()
}

/// List the immediate children of `prefix` (one level, non-recursive).
pub async fn list_dir(store: &dyn ObjectStore, prefix: &ObjPath) -> Result<DirListing> {
    let result = store.list_with_delimiter(Some(prefix)).await?;
    Ok(DirListing {
        files: result
            .objects
            .iter()
            .map(|o| basename(&o.location))
            .collect(),
        subdirs: result.common_prefixes.iter().map(basename).collect(),
    })
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
/// object recursively under `prefix`. Used for `Mixed`/`Unknown` aux dirs where we don't
/// otherwise inspect content (design doc §3.3).
pub async fn fingerprint(store: &dyn ObjectStore, prefix: &ObjPath) -> Result<String> {
    let mut stream = store.list(Some(prefix));
    let mut tuples: Vec<(String, String, u64)> = Vec::new();
    while let Some(meta) = stream.try_next().await? {
        tuples.push((
            meta.location.as_ref().to_string(),
            meta.e_tag.clone().unwrap_or_default(),
            meta.size,
        ));
    }
    tuples.sort();
    let mut hasher = DefaultHasher::new();
    tuples.hash(&mut hasher);
    Ok(format!("{:016x}", hasher.finish()))
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
pub async fn detect_format(
    store: &dyn ObjectStore,
    prefix: &ObjPath,
) -> Result<(AuxFormat, Option<String>)> {
    let listing = list_dir(store, prefix).await?;

    let has_success = listing.files.iter().any(|f| f == "_SUCCESS");
    let has_parquet_part = listing
        .files
        .iter()
        .any(|f| f.starts_with("part-") && f.ends_with(".parquet"));
    if has_success && has_parquet_part {
        return Ok((AuxFormat::Parquet, None));
    }

    if is_lance_shaped(&listing) {
        return Ok((AuxFormat::Lance, None));
    }

    if listing.files.iter().any(|f| f.ends_with(".csv")) {
        return Ok((AuxFormat::Csv, None));
    }

    let fp = fingerprint(store, prefix).await?;
    if listing.files.is_empty() && listing.subdirs.is_empty() {
        Ok((AuxFormat::Unknown, Some(fp)))
    } else {
        Ok((AuxFormat::Mixed, Some(fp)))
    }
}
