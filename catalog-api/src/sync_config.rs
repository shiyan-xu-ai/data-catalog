//! Build a `catalog_store::SyncConfig` from a root URI: `s3://bucket/prefix` in prod, or a plain
//! filesystem path for local dev and tests.
//!
//! For `s3://` URIs the object store is built with `object_store::parse_url_opts` feeding in every
//! `AWS_*` environment variable recognized by `AmazonS3ConfigKey`, plus any credentials the caller
//! hydrated from Secret Manager (see [`crate::secrets`]). The env vars are what make the local
//! overlay's MinIO endpoint (`AWS_ENDPOINT_URL`), `AWS_ALLOW_HTTP`, and path-style flag reach the
//! S3 client — a bare `parse_url` (empty opts) builds the client via `AmazonS3Builder::new()`,
//! which reads NO environment, so that config would be silently dropped. On Cloud Run the access
//! keys come from Secret Manager (there is no ambient AWS credential) and region/endpoint come
//! from plain `[cloudrun].env_vars`.

use std::str::FromStr;
use std::sync::Arc;

use anyhow::{Context, Result};
use catalog_store::SyncConfig;
use object_store::local::LocalFileSystem;
use object_store::path::Path as ObjPath;

/// Collect the `AWS_*` environment variables recognized by `object_store`'s
/// [`AmazonS3ConfigKey`](object_store::aws::AmazonS3ConfigKey) into a `Vec<(String, String)>`
/// suitable for [`object_store::parse_url_opts`].
///
/// `parse_url_opts` folds each option into the builder via
/// `AmazonS3Builder::with_config(key, value)`, matching keys case-insensitively through
/// `AmazonS3ConfigKey::from_str` (which accepts both `aws_*` and bare names). Only variables
/// that parse to a recognized config key are yielded; unknown `AWS_*` names are skipped
/// (matching `from_env()`'s behavior). The env var NAMES this honors must stay in sync with
/// object_store 0.13.2's `AmazonS3ConfigKey::from_str`; the load-bearing ones for the local
/// overlay are `AWS_ENDPOINT_URL`/`AWS_ENDPOINT`, `AWS_ACCESS_KEY_ID`,
/// `AWS_SECRET_ACCESS_KEY`, `AWS_ALLOW_HTTP`, `AWS_VIRTUAL_HOSTED_STYLE_REQUEST`, and
/// `AWS_DEFAULT_REGION`.
fn aws_s3_opts_from_env() -> Vec<(String, String)> {
    let mut opts = Vec::new();
    for (os_key, os_value) in std::env::vars_os() {
        let (Some(key), Some(value)) = (os_key.to_str(), os_value.to_str()) else {
            continue;
        };
        if !key.starts_with("AWS_") {
            continue;
        }
        // parse_url_opts lowercases the key before FromStr-dispatching it; mirror that here so
        // the filter (recognized key?) matches exactly what the builder would apply.
        if object_store::aws::AmazonS3ConfigKey::from_str(&key.to_ascii_lowercase()).is_ok() {
            opts.push((key.to_string(), value.to_string()));
        }
    }
    opts
}

/// The full S3 option set: `AWS_*` env vars plus any credentials hydrated from Secret Manager
/// (`extra`, appended last so a Secret Manager value wins on the rare key overlap).
fn s3_opts(extra: &[(String, String)]) -> Vec<(String, String)> {
    let mut opts = aws_s3_opts_from_env();
    opts.extend(extra.iter().cloned());
    opts
}

pub fn build_sync_config(root_uri: &str, extra_opts: &[(String, String)]) -> Result<SyncConfig> {
    if root_uri.contains("://") {
        let url = url::Url::parse(root_uri).context("parse sync root URI")?;
        let (store, path) = object_store::parse_url_opts(&url, s3_opts(extra_opts))
            .context("build object store for sync root")?;
        Ok(SyncConfig::new(
            Arc::from(store),
            path,
            root_uri.to_string(),
        ))
    } else {
        let store = LocalFileSystem::new_with_prefix(root_uri)
            .with_context(|| format!("open local filesystem sync root {root_uri}"))?;
        Ok(SyncConfig::new(
            Arc::new(store),
            ObjPath::from(""),
            root_uri.to_string(),
        ))
    }
}

/// Build the authored-overlay [`MetaStore`] from `uri`. Because the overlay is mutated with
/// conditional writes (ETag CAS), the backing store must support them:
/// - `memory` — an `InMemory` store (dev / local without MinIO / tests; NOT persistent).
/// - `s3://bucket/prefix` — S3 or a MinIO-compatible endpoint (production, local dev). AWS_*
///   env is applied exactly as for the sync store.
///
/// A plain filesystem path is rejected: `object_store`'s `LocalFileSystem` does not implement
/// `PutMode::Update`, so overlay mutations would fail at runtime.
pub fn build_meta_store(
    uri: &str,
    extra_opts: &[(String, String)],
) -> Result<catalog_store::MetaStore> {
    if uri == "memory" {
        return Ok(catalog_store::MetaStore::new(
            Arc::new(object_store::memory::InMemory::new()),
            ObjPath::from(""),
        ));
    }
    if uri.contains("://") {
        let url = url::Url::parse(uri).context("parse overlay base URI")?;
        let (store, path) = object_store::parse_url_opts(&url, s3_opts(extra_opts))
            .context("build object store for overlay")?;
        return Ok(catalog_store::MetaStore::new(Arc::from(store), path));
    }
    anyhow::bail!(
        "CATALOG_META_BASE_URI must be `memory` or an s3://... URI (LocalFileSystem does not \
         support the conditional writes the overlay needs), got {uri:?}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `aws_s3_opts_from_env` must surface exactly the `AWS_*` env vars object_store 0.13.2
    /// recognizes via `AmazonS3ConfigKey::from_str`, and must NOT emit unrecognized names
    /// (e.g. a bogus `AWS_S3_FORCE_PATH_STYLE` — the real path-style key is
    /// `AWS_VIRTUAL_HOSTED_STYLE_REQUEST`). This is the pure env→options mapping the sync
    /// store build depends on; a wrong key name here is the same silent-misconfig bug the fix
    /// targets, so it is unit-tested in isolation.
    #[test]
    fn aws_s3_opts_from_env_picks_up_recognized_keys_and_skips_unknown() {
        // Set a recognized endpoint var + a recognized allow_http var + a bogus var object_store
        // 0.13.2 does NOT recognize (the old overlay used this bogus name; the fix renamed it).
        // SAFETY: env mutation in a single-threaded unit test is acceptable for this check.
        // These run in the test process; parallel tests in this binary are not affected because
        // they do not read these specific vars.
        unsafe {
            std::env::set_var("AWS_ENDPOINT_URL", "http://minio:9000");
            std::env::set_var("AWS_ALLOW_HTTP", "true");
            std::env::set_var("AWS_VIRTUAL_HOSTED_STYLE_REQUEST", "false");
            std::env::set_var("AWS_S3_FORCE_PATH_STYLE", "true"); // unrecognized -> skipped
        }
        let opts = aws_s3_opts_from_env();
        unsafe {
            std::env::remove_var("AWS_ENDPOINT_URL");
            std::env::remove_var("AWS_ALLOW_HTTP");
            std::env::remove_var("AWS_VIRTUAL_HOSTED_STYLE_REQUEST");
            std::env::remove_var("AWS_S3_FORCE_PATH_STYLE");
        }

        let as_map: std::collections::HashMap<String, String> = opts.into_iter().collect();
        assert_eq!(
            as_map.get("AWS_ENDPOINT_URL").map(|s| s.as_str()),
            Some("http://minio:9000"),
            "AWS_ENDPOINT_URL must be picked up (object_store maps aws_endpoint_url -> Endpoint)"
        );
        assert_eq!(
            as_map.get("AWS_ALLOW_HTTP").map(|s| s.as_str()),
            Some("true"),
            "AWS_ALLOW_HTTP must be picked up (object_store maps aws_allow_http -> Client(AllowHttp))"
        );
        assert_eq!(
            as_map.get("AWS_VIRTUAL_HOSTED_STYLE_REQUEST").map(|s| s.as_str()),
            Some("false"),
            "AWS_VIRTUAL_HOSTED_STYLE_REQUEST=false must be picked up (disables virtual-host = path-style)"
        );
        assert!(
            !as_map.contains_key("AWS_S3_FORCE_PATH_STYLE"),
            "AWS_S3_FORCE_PATH_STYLE is NOT a recognized object_store 0.13.2 key and must be \
             skipped — the local overlay must use AWS_VIRTUAL_HOSTED_STYLE_REQUEST=false instead"
        );
    }
}
