//! Shared fixtures for the `catalog-api` integration tests. Included per test binary via
//! `mod common;`. Different binaries use different subsets, so unused-helper warnings are
//! expected and allowed here (each binary only compiles the helpers it references anyway).
#![allow(dead_code)]

use std::path::Path;
use std::sync::Arc;

use catalog_store::SyncConfig;
use object_store::local::LocalFileSystem;
use object_store::path::Path as ObjPath;

/// A `SyncConfig` backed by a local filesystem rooted at `root` (empty root prefix). This is
/// the one builder every API/sync integration test used to hand-roll under a different name.
pub fn sync_config(root: &Path) -> SyncConfig {
    let store = Arc::new(LocalFileSystem::new_with_prefix(root).unwrap());
    SyncConfig::new(store, ObjPath::from(""), root.to_str().unwrap().to_string())
}
