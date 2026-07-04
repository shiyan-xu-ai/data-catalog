//! Background task that periodically re-reads the `_catalog/registry` Lance table and
//! caches the result in shared state. Every pod (leader or not) runs this, so non-leader
//! replicas pick up the leader's sweep writes without needing their own write access.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use catalog_core::TableEntry;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

/// Shared, cheaply-cloneable cache of the latest registry read.
pub type RegistryCache = Arc<RwLock<Vec<TableEntry>>>;

/// Set to `true` after the first successful registry read (whether the registry exists yet or
/// not), i.e. once storage is reachable and the cache reflects it. Drives the `/readyz` probe:
/// distinct from "the cache has tables", so a legitimately empty catalog still reports ready.
pub type Hydrated = Arc<AtomicBool>;

/// Spawn the refresh loop. Read failures (e.g. a transient object-store error) are logged and
/// skipped rather than propagated — the cache stays at its previous value until a read succeeds.
/// `hydrated` is flipped to `true` on the first successful read (existing registry or not).
pub fn spawn_refresh(
    registry_path: String,
    cache: RegistryCache,
    hydrated: Hydrated,
    interval: Duration,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            match catalog_core::read_registry(&registry_path).await {
                Ok(Some(entries)) => {
                    *cache.write().await = entries;
                    hydrated.store(true, Ordering::SeqCst);
                }
                // Registry not written yet (before the first sweep): keep the current (empty)
                // cache, but storage is reachable, so we are hydrated (empty-but-ready).
                Ok(None) => {
                    hydrated.store(true, Ordering::SeqCst);
                }
                Err(e) => {
                    tracing::debug!(error = %e, "registry refresh: read failed, keeping cached state");
                }
            }
            tokio::time::sleep(interval).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use catalog_core::{Namespace, TableEntry};

    fn sample_entry(id: &str) -> TableEntry {
        TableEntry {
            id: id.to_string(),
            name: id.to_string(),
            namespace: Namespace::new(["ns"]),
            root_location: format!("s3://bucket/{id}"),
            owner: None,
            ttl_policy: None,
            last_swept: None,
            versions: vec![],
            aux_latest: vec![],
        }
    }

    #[tokio::test]
    async fn refresh_loop_picks_up_a_registry_written_after_it_starts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("registry.lance");
        let path = path.to_str().unwrap().to_string();

        let cache: RegistryCache = Arc::new(RwLock::new(Vec::new()));
        let hydrated: Hydrated = Arc::new(AtomicBool::new(false));
        let handle = spawn_refresh(
            path.clone(),
            cache.clone(),
            hydrated.clone(),
            Duration::from_millis(20),
        );

        // Nothing written yet: cache stays empty, but a successful read (registry-absent) still
        // marks the pod hydrated (storage reachable, empty-but-ready).
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(cache.read().await.is_empty());
        assert!(
            hydrated.load(Ordering::SeqCst),
            "a successful registry-absent read must mark the pod hydrated"
        );

        // Write the registry "out of band" (as the leader's sweep loop would).
        catalog_core::write_registry(&path, &[sample_entry("t1")])
            .await
            .unwrap();

        // Wait for at least one refresh tick to pick it up.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let cached = cache.read().await;
        assert_eq!(cached.len(), 1);
        assert_eq!(cached[0].id, "t1");

        handle.abort();
    }
}
