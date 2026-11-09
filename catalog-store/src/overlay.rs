//! Authored-metadata overlay: the small, human-mutated slice of catalog state, stored as one
//! JSON object per table under `_catalog/meta/<table_id>.json` and mutated with object-store
//! conditional writes (ETag compare-and-set) instead of a lock or leader.
//! NOTE: `object_store`'s `LocalFileSystem` does not implement `PutMode::Update`, so this store
//! must be backed by S3 / a MinIO-compatible endpoint (production, local dev) or `InMemory`
//! (tests). The sync/data path still uses `LocalFileSystem` freely — it does no conditional
//! writes.

use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use object_store::path::Path as ObjPath;
use object_store::{ObjectStore, ObjectStoreExt, PutMode, PutPayload, UpdateVersion};
use serde::{Deserialize, Serialize};

const MAX_ATTEMPTS: usize = 16;

/// The authored slice of one table's catalog state. Everything here is human-set via the API;
/// the sync never writes it. Absent fields default (forward/backward compatible, like the
/// registry types).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableMeta {
    #[serde(default)]
    pub owner: Option<String>,
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
    async fn concurrent_owner_mutations_survive_via_cas_retry() {
        let ms = store();
        ms.mutate_meta("t1", |m| m.owner = Some(String::new()))
            .await
            .unwrap();

        let (a, b) = tokio::join!(
            ms.mutate_meta("t1", |m| m.owner.as_mut().unwrap().push('a')),
            ms.mutate_meta("t1", |m| m.owner.as_mut().unwrap().push('b')),
        );
        a.unwrap();
        b.unwrap();

        let owner = ms.get_meta("t1").await.unwrap().unwrap().owner.unwrap();
        assert!(
            owner == "ab" || owner == "ba",
            "both mutations survive: {owner}"
        );
    }

    #[tokio::test]
    async fn older_registration_json_preserves_owner_and_empty_registration() {
        let store = Arc::new(InMemory::new());
        let ms = MetaStore::new(store.clone(), ObjPath::from("_catalog/meta"));
        for (id, json) in [
            (
                "owned",
                r#"{"owner":"alice","ttl_policy":{"keep_last_n":3},"protected":["v1"],"deleting":["v2"]}"#,
            ),
            (
                "empty",
                r#"{"ttl_policy":null,"protected":[],"deleting":[]}"#,
            ),
        ] {
            store
                .put(&ms.path(id), PutPayload::from(json.as_bytes().to_vec()))
                .await
                .unwrap();
        }

        assert_eq!(
            ms.get_meta("owned")
                .await
                .unwrap()
                .unwrap()
                .owner
                .as_deref(),
            Some("alice")
        );
        assert_eq!(
            ms.get_meta("empty").await.unwrap(),
            Some(TableMeta::default())
        );
        assert_eq!(ms.list_meta().await.unwrap().len(), 2);

        ms.mutate_meta("owned", |m| m.owner = Some("bob".into()))
            .await
            .unwrap();
        let bytes = store
            .get(&ms.path("owned"))
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        let written: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(written, serde_json::json!({"owner": "bob"}));
    }
}
