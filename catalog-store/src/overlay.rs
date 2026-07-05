//! Authored-metadata overlay: the small, human-mutated slice of catalog state, stored as one
//! JSON object per table under `_catalog/meta/<table_id>.json` and mutated with object-store
//! conditional writes (ETag compare-and-set) instead of a lock or leader.
//!
//! ## Why an overlay
//!
//! Registry state splits in two by how it is produced. **Derived** fields (versions, shapes,
//! byte splits, row counts, schemas, aux) are a pure function of immutable S3 content — any
//! sweep re-derives them, so the derived snapshot is written whole, last-wins, with no
//! coordination. **Authored** fields (`owner`, `ttl_policy`, per-version `protected`) are the
//! only thing humans mutate; this overlay holds exactly those, keyed per table, so a mutation
//! touches a few hundred bytes and races are resolved by S3's own conditional write rather than
//! an application lock.
//!
//! ## Concurrency
//!
//! `mutate_meta` is the read-modify-write primitive: GET the object (or start fresh if absent),
//! apply the caller's closure, then `PutMode::Update` pinned to the observed ETag (or
//! `PutMode::Create` if it was absent). If another writer committed in between, the store
//! returns `Precondition`/`AlreadyExists`; we re-read and re-apply the closure on the fresh
//! state, bounded by `MAX_ATTEMPTS`. At the catalog's admin write volume contention is
//! effectively never, but the retry makes it correct regardless.
//!
//! NOTE: `object_store`'s `LocalFileSystem` does not implement `PutMode::Update`, so this store
//! must be backed by S3 / a MinIO-compatible endpoint (production, local dev) or `InMemory`
//! (tests). The sweep/data path still uses `LocalFileSystem` freely — it does no conditional
//! writes.

use std::collections::BTreeSet;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use catalog_core::TtlPolicy;
use object_store::path::Path as ObjPath;
use object_store::{ObjectStore, ObjectStoreExt, PutMode, PutPayload, UpdateVersion};
use serde::{Deserialize, Serialize};

/// Bound on conditional-write retries before giving up (surfaces as an error the caller maps to
/// a 500; a client can retry). Contention here is admin-scale, so this is only a safety ceiling.
const MAX_ATTEMPTS: usize = 16;

/// The authored slice of one table's catalog state. Everything here is human-set via the API;
/// the sweep never writes it. Absent fields default (forward/backward compatible, like the
/// registry types).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableMeta {
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub ttl_policy: Option<TtlPolicy>,
    /// Version ids the operator has marked TTL-exempt.
    #[serde(default)]
    pub protected: BTreeSet<String>,
    /// Version ids a TTL apply has begun deleting (the pending-delete marker that lets a
    /// crashed apply resume idempotently and proves a snapshot postdates any concurrent
    /// `protect`). Empty in steady state.
    #[serde(default)]
    pub deleting: BTreeSet<String>,
}

impl TableMeta {
    /// True when the overlay carries no authored state (safe to treat as absent).
    pub fn is_empty(&self) -> bool {
        self.owner.is_none()
            && self.ttl_policy.is_none()
            && self.protected.is_empty()
            && self.deleting.is_empty()
    }
}

/// Per-table authored-overlay store over an object store that supports conditional writes.
#[derive(Clone)]
pub struct MetaStore {
    store: Arc<dyn ObjectStore>,
    base: ObjPath,
}

impl MetaStore {
    /// `base` is the prefix the per-table objects live under (e.g. `_catalog/meta`).
    pub fn new(store: Arc<dyn ObjectStore>, base: ObjPath) -> Self {
        Self { store, base }
    }

    fn path(&self, table_id: &str) -> ObjPath {
        let base = self.base.as_ref().trim_end_matches('/');
        if base.is_empty() {
            ObjPath::from(format!("{table_id}.json"))
        } else {
            ObjPath::from(format!("{base}/{table_id}.json"))
        }
    }

    /// Read one table's overlay, or `None` if it has never been written.
    pub async fn get_meta(&self, table_id: &str) -> Result<Option<TableMeta>> {
        let path = self.path(table_id);
        match self.store.get(&path).await {
            Ok(result) => {
                let bytes = result.bytes().await.context("read overlay object")?;
                let meta = serde_json::from_slice(&bytes).context("deserialize overlay")?;
                Ok(Some(meta))
            }
            Err(object_store::Error::NotFound { .. }) => Ok(None),
            Err(e) => Err(e).context("get overlay object"),
        }
    }

    /// Load every table's overlay under `base`, keyed by table id. Used by read handlers to
    /// merge authored state over the derived snapshot (small: one object per table).
    pub async fn list_meta(&self) -> Result<std::collections::HashMap<String, TableMeta>> {
        use futures::TryStreamExt;
        let mut out = std::collections::HashMap::new();
        let mut listing = self.store.list(Some(&self.base));
        while let Some(obj) = listing.try_next().await.context("list overlays")? {
            let Some(file) = obj.location.filename() else {
                continue;
            };
            let Some(table_id) = file.strip_suffix(".json") else {
                continue;
            };
            if let Some(meta) = self.get_meta(table_id).await? {
                out.insert(table_id.to_string(), meta);
            }
        }
        Ok(out)
    }

    /// Conditionally read-modify-write one table's overlay. See the module docs for the CAS
    /// retry contract. The closure may run more than once (on contention) and must be
    /// idempotent in intent; it is always applied to freshly-read state before each commit.
    pub async fn mutate_meta<F>(&self, table_id: &str, mut apply: F) -> Result<TableMeta>
    where
        F: FnMut(&mut TableMeta),
    {
        let path = self.path(table_id);
        for _ in 0..MAX_ATTEMPTS {
            let (mut meta, mode) = match self.store.get(&path).await {
                Ok(result) => {
                    let version = UpdateVersion {
                        e_tag: result.meta.e_tag.clone(),
                        version: result.meta.version.clone(),
                    };
                    let bytes = result.bytes().await.context("read overlay for mutate")?;
                    let meta = serde_json::from_slice(&bytes).context("deserialize overlay")?;
                    (meta, PutMode::Update(version))
                }
                Err(object_store::Error::NotFound { .. }) => {
                    (TableMeta::default(), PutMode::Create)
                }
                Err(e) => return Err(e).context("get overlay for mutate"),
            };

            apply(&mut meta);
            let body = serde_json::to_vec(&meta).context("serialize overlay")?;
            match self
                .store
                .put_opts(&path, PutPayload::from(body), mode.into())
                .await
            {
                Ok(_) => return Ok(meta),
                // Another writer committed since our read: re-read and re-apply.
                Err(object_store::Error::Precondition { .. })
                | Err(object_store::Error::AlreadyExists { .. }) => continue,
                Err(e) => return Err(e).context("conditional put overlay"),
            }
        }
        Err(anyhow!(
            "overlay CAS for {table_id} exhausted {MAX_ATTEMPTS} attempts under contention"
        ))
    }

    /// Delete a table's overlay entirely (used when the table is deregistered).
    pub async fn delete_meta(&self, table_id: &str) -> Result<()> {
        match self.store.delete(&self.path(table_id)).await {
            Ok(()) => Ok(()),
            Err(object_store::Error::NotFound { .. }) => Ok(()),
            Err(e) => Err(e).context("delete overlay object"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use object_store::memory::InMemory;

    fn store() -> MetaStore {
        MetaStore::new(Arc::new(InMemory::new()), ObjPath::from("_catalog/meta"))
    }

    #[tokio::test]
    async fn create_get_and_delete_round_trip() {
        let ms = store();
        assert!(ms.get_meta("t1").await.unwrap().is_none());

        ms.mutate_meta("t1", |m| m.owner = Some("alice".into()))
            .await
            .unwrap();
        assert_eq!(
            ms.get_meta("t1").await.unwrap().unwrap().owner.as_deref(),
            Some("alice")
        );

        ms.delete_meta("t1").await.unwrap();
        assert!(ms.get_meta("t1").await.unwrap().is_none());
        // Delete of an absent overlay is a no-op, not an error.
        ms.delete_meta("t1").await.unwrap();
    }

    #[tokio::test]
    async fn concurrent_mutations_both_survive_via_cas_retry() {
        let ms = store();
        ms.mutate_meta("t1", |m| m.owner = Some("alice".into()))
            .await
            .unwrap();

        // Two independent authored changes race on the same overlay object; the CAS retry must
        // land both without either clobbering the other or the pre-existing owner.
        let (a, b) = tokio::join!(
            ms.mutate_meta("t1", |m| {
                m.protected.insert("v1".into());
            }),
            ms.mutate_meta("t1", |m| {
                m.ttl_policy = Some(TtlPolicy {
                    keep_last_n: Some(3),
                    max_age_days: None,
                });
            }),
        );
        a.unwrap();
        b.unwrap();

        let final_meta = ms.get_meta("t1").await.unwrap().unwrap();
        assert_eq!(
            final_meta.owner.as_deref(),
            Some("alice"),
            "owner preserved"
        );
        assert!(final_meta.protected.contains("v1"), "protect landed");
        assert_eq!(
            final_meta.ttl_policy.unwrap().keep_last_n,
            Some(3),
            "ttl policy landed"
        );
    }
}
