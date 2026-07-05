//! Sweep configuration: which object store + root prefix to sweep, and how to build the
//! full URI lance needs to open a dataset.

use std::sync::Arc;

use anyhow::{anyhow, Result};
use object_store::path::Path as ObjPath;
use object_store::ObjectStore;

/// Configuration for one sweep pass.
///
/// `store` is used for all LIST operations (discovery + byte accounting); it is generic
/// over any `object_store::ObjectStore` impl so the same sweep code runs against
/// `LocalFileSystem`/`InMemory` in tests and against S3 (or a MinIO-compatible endpoint) in
/// production. `root_path` is the prefix within `store` that is swept. `root_uri` is the
/// URI prefix lance's `Dataset::open` needs to resolve the *same* location — for local tests
/// this is a plain filesystem path, for S3 it is `s3://bucket/prefix`.
#[derive(Clone)]
pub struct SweepConfig {
    pub store: Arc<dyn ObjectStore>,
    pub root_path: ObjPath,
    pub root_uri: String,
    /// How many versions sweep concurrently (in-flight LISTs + dataset opens). The sweep is
    /// S3-latency-bound, so wall time ≈ `versions / concurrency`.
    pub concurrency: usize,
}

/// Default in-flight version sweeps. S3-class stores comfortably serve far more concurrent
/// requests than this; the cap bounds memory (one version's object list in flight per slot).
pub const DEFAULT_SWEEP_CONCURRENCY: usize = 16;

impl SweepConfig {
    pub fn new(
        store: Arc<dyn ObjectStore>,
        root_path: ObjPath,
        root_uri: impl Into<String>,
    ) -> Self {
        Self {
            store,
            root_path,
            root_uri: root_uri.into(),
            concurrency: DEFAULT_SWEEP_CONCURRENCY,
        }
    }

    /// Override the version-sweep concurrency (clamped to at least 1).
    pub fn with_concurrency(mut self, concurrency: usize) -> Self {
        self.concurrency = concurrency.max(1);
        self
    }

    /// Build the full URI lance needs to open the dataset at `full_path`, a full
    /// object-store path (including `root_path`'s prefix).
    pub fn uri_for(&self, full_path: &ObjPath) -> String {
        let root_uri = self.root_uri.trim_end_matches('/');
        let root_str = self.root_path.as_ref().trim_end_matches('/');
        let full_str = full_path.as_ref();
        let rel = full_str
            .strip_prefix(root_str)
            .unwrap_or(full_str)
            .trim_start_matches('/');
        if rel.is_empty() {
            root_uri.to_string()
        } else {
            format!("{root_uri}/{rel}")
        }
    }

    /// Inverse of `uri_for`: recover the `ObjectStore`-relative path from a URI previously
    /// produced by `uri_for` (e.g. a `TableVersion::snapshot_path`), so it can be used for
    /// LIST/DELETE against `store` (TTL hard-delete).
    ///
    /// Errors if `uri` is not under `root_uri`. This is a hard error rather than a silent
    /// fallback: `path_for` feeds the irreversible TTL delete, and a fallback path that doesn't
    /// resolve to real objects would turn a failed delete into a fake success (the version
    /// dropped from the registry while its bytes remain on S3).
    pub fn path_for(&self, uri: &str) -> Result<ObjPath> {
        let root_uri = self.root_uri.trim_end_matches('/');
        let rel = uri
            .strip_prefix(root_uri)
            .ok_or_else(|| anyhow!("uri {uri} is not under sweep root {root_uri}"))?
            .trim_start_matches('/');
        let root_str = self.root_path.as_ref().trim_end_matches('/');
        if rel.is_empty() {
            Ok(self.root_path.clone())
        } else if root_str.is_empty() {
            Ok(ObjPath::from(rel))
        } else {
            Ok(ObjPath::from(format!("{root_str}/{rel}")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use object_store::local::LocalFileSystem;
    use std::sync::Arc;

    #[test]
    fn path_for_is_the_inverse_of_uri_for() {
        let cfg = SweepConfig::new(
            Arc::new(LocalFileSystem::new()),
            ObjPath::from("scenario_dataset_export"),
            "s3://bucket/scenario_dataset_export".to_string(),
        );
        let original = ObjPath::from("scenario_dataset_export/smoke_test/2026-06-26_12-00-00");
        let uri = cfg.uri_for(&original);
        assert_eq!(
            uri,
            "s3://bucket/scenario_dataset_export/smoke_test/2026-06-26_12-00-00"
        );
        assert_eq!(cfg.path_for(&uri).unwrap(), original);
    }

    #[test]
    fn path_for_errors_when_uri_is_not_under_the_sweep_root() {
        let cfg = SweepConfig::new(
            Arc::new(LocalFileSystem::new()),
            ObjPath::from("scenario_dataset_export"),
            "s3://bucket/scenario_dataset_export".to_string(),
        );
        // A snapshot_path pointing at a different bucket/root must NOT silently fall back to a
        // path under this store -- it must error, so a TTL delete refuses rather than pretending
        // to have deleted something it never located.
        assert!(cfg.path_for("s3://other-bucket/elsewhere/v1").is_err());
    }
}
