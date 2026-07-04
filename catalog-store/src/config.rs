//! Sweep configuration: which object store + root prefix to sweep, and how to build the
//! full URI lance needs to open a dataset.

use std::sync::Arc;

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
}

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
        }
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
}
