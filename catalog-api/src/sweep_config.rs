//! Build a `catalog_store::SweepConfig` from a root URI: `s3://bucket/prefix` in prod (via
//! IRSA/env credentials), or a plain filesystem path for local dev and tests.

use std::sync::Arc;

use anyhow::{Context, Result};
use catalog_store::SweepConfig;
use object_store::local::LocalFileSystem;
use object_store::path::Path as ObjPath;

pub fn build_sweep_config(root_uri: &str) -> Result<SweepConfig> {
    if root_uri.contains("://") {
        let url = url::Url::parse(root_uri).context("parse sweep root URI")?;
        let (store, path) =
            object_store::parse_url(&url).context("build object store for sweep root")?;
        Ok(SweepConfig::new(
            Arc::from(store),
            path,
            root_uri.to_string(),
        ))
    } else {
        let store = LocalFileSystem::new_with_prefix(root_uri)
            .with_context(|| format!("open local filesystem sweep root {root_uri}"))?;
        Ok(SweepConfig::new(
            Arc::new(store),
            ObjPath::from(""),
            root_uri.to_string(),
        ))
    }
}
