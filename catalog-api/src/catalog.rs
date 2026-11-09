//! Read model: the merged catalog view (derived snapshot + authored overlays) with lazy
//! revalidation.
//!
//! State is split by how it is produced (see `catalog_store::overlay`): the **derived snapshot**
//! (`_catalog/registry`, a Lance dataset) is written whole by the sync and holds versions,
//! shapes, sizes, schemas, aux — everything recomputable from immutable S3 content. The
//! **authored overlays** (`_catalog/meta/<id>.json`) hold registration and owner metadata. A read
//! merges the two back into the `TableEntry` wire shape the API and frontend already expect.
//!
//! Cloud Run throttles CPU between requests, so there is no background refresh loop: the view is
//! revalidated lazily — re-read from storage when older than `ttl`, and immediately after a
//! mutation invalidates it (read-your-writes on the same instance). On a revalidation error the
//! last good view is kept and served (staleness is bounded by the sync cadence anyway).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use catalog_core::{Namespace, TableEntry};
use catalog_store::{MetaStore, TableMeta};
use tokio::sync::{Mutex, RwLock};

/// Merge the authored overlays over the derived snapshot into the wire-shape `TableEntry` list.
///
/// Snapshot entries carry no authored state (the sync never writes it), so the overlay is
/// authoritative for `owner`. Tables registered before their first sync have no derived metadata.
/// Output is sorted by id for stable listing.
pub fn merge(snapshot: Vec<TableEntry>, overlays: HashMap<String, TableMeta>) -> Vec<TableEntry> {
    let mut by_id: HashMap<String, TableEntry> =
        snapshot.into_iter().map(|e| (e.id.clone(), e)).collect();
    for (id, meta) in overlays {
        let entry = by_id.entry(id.clone()).or_insert_with(|| stub_entry(&id));
        entry.owner = meta.owner;
    }
    let mut out: Vec<TableEntry> = by_id.into_values().collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

fn stub_entry(id: &str) -> TableEntry {
    let parsed = catalog_core::parse_table_id(id);
    let (region, bucket, namespace, name) = match parsed {
        Some(p) => (p.region, p.bucket, p.namespace, p.name),
        None => (String::new(), String::new(), Vec::new(), id.to_string()),
    };
    TableEntry {
        id: id.to_string(),
        name,
        region,
        bucket,
        namespace: Namespace::new(namespace),
        root_location: String::new(),
        owner: None,
        last_synced: None,
        versions: Vec::new(),
        aux_latest: Vec::new(),
    }
}

struct CacheState {
    view: Vec<TableEntry>,
    loaded_at: Option<Instant>,
}

/// The catalog read model, shared (`Arc`) across handlers.
pub struct Catalog {
    registry_path: String,
    meta: MetaStore,
    ttl: Duration,
    state: RwLock<CacheState>,
    /// Serializes revalidation so a burst of stale reads triggers one reload, not many.
    reload_lock: Mutex<()>,
}

impl Catalog {
    pub fn new(registry_path: String, meta: MetaStore, ttl: Duration) -> Arc<Self> {
        Arc::new(Self {
            registry_path,
            meta,
            ttl,
            state: RwLock::new(CacheState {
                view: Vec::new(),
                loaded_at: None,
            }),
            reload_lock: Mutex::new(()),
        })
    }

    /// Read the snapshot + all overlays and rebuild the merged view.
    async fn load(&self) -> Result<Vec<TableEntry>> {
        let snapshot = catalog_core::read_registry(&self.registry_path)
            .await?
            .unwrap_or_default();
        let overlays = self.meta.list_meta().await?;
        Ok(merge(snapshot, overlays))
    }

    /// Return the merged view, revalidating first if it is missing or older than `ttl`. On a
    /// revalidation error the previous view is kept and returned (with a warning) rather than
    /// failing the read.
    pub async fn view(&self) -> Vec<TableEntry> {
        let fresh = {
            let s = self.state.read().await;
            s.loaded_at.is_some_and(|t| t.elapsed() < self.ttl)
        };
        if !fresh {
            let _reload = self.reload_lock.lock().await;
            // Re-check: another task may have reloaded while we waited for the lock.
            let still_stale = {
                let s = self.state.read().await;
                s.loaded_at.is_none_or(|t| t.elapsed() >= self.ttl)
            };
            if still_stale {
                match self.load().await {
                    Ok(view) => {
                        let mut s = self.state.write().await;
                        s.view = view;
                        s.loaded_at = Some(Instant::now());
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "catalog revalidation failed; serving last view");
                    }
                }
            }
        }
        self.state.read().await.view.clone()
    }

    /// Force the next `view()` to revalidate (called after a mutation for read-your-writes).
    pub async fn invalidate(&self) {
        self.state.write().await.loaded_at = None;
    }

    /// True once the view has loaded at least once (drives `/readyz`).
    pub async fn is_ready(&self) -> bool {
        self.state.read().await.loaded_at.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use catalog_core::{TableVersion, VersionShape};

    fn version(id: &str) -> TableVersion {
        TableVersion {
            version_id: id.to_string(),
            timestamp: chrono::Utc::now(),
            snapshot_path: format!("s3://b/t/{id}"),
            shape: VersionShape::Full,
            partial: false,
            storage_bytes_total: 0,
            object_count: None,
            lance_core_bytes: 0,
            sidecar_bytes: 0,
            segments_bytes: 0,
            other_aux_bytes: 0,
            row_count: None,
            num_fragments: None,
            schema_json: None,
            num_indices: None,
            lance_version: None,
            writer_version: None,
            aux: vec![],
            synced_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn merge_overlays_authored_fields_and_materializes_overlay_only_stubs() {
        let mut snap_entry = stub_entry("synced");
        snap_entry.root_location = "s3://b/synced".into();
        snap_entry.versions = vec![version("v1"), version("v2")];

        let mut overlays = HashMap::new();
        overlays.insert(
            "synced".to_string(),
            TableMeta {
                owner: Some("alice".into()),
            },
        );
        // An overlay for a table the sync hasn't observed yet -> stub in the view.
        overlays.insert(
            "declared_only".to_string(),
            TableMeta {
                owner: Some("bob".into()),
            },
        );

        let view = merge(vec![snap_entry], overlays);
        assert_eq!(view.len(), 2);

        let synced = view.iter().find(|e| e.id == "synced").unwrap();
        assert_eq!(synced.owner.as_deref(), Some("alice"));
        assert_eq!(synced.versions.len(), 2);
        assert_eq!(synced.versions[0].version_id, "v1");
        assert_eq!(synced.versions[1].version_id, "v2");

        let declared = view.iter().find(|e| e.id == "declared_only").unwrap();
        assert_eq!(declared.owner.as_deref(), Some("bob"));
        assert!(declared.versions.is_empty());
    }
}
