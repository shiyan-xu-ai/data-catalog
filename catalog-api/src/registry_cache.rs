//! Background task that periodically re-reads the `_catalog/registry` Lance table and
//! caches the result in shared state. Every pod (leader or not) runs this, so non-leader
//! replicas pick up the leader's sweep writes without needing their own write access.

use std::sync::Arc;
use std::time::Duration;

use catalog_core::TableEntry;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

/// Shared, cheaply-cloneable cache of the latest registry read.
pub type RegistryCache = Arc<RwLock<Vec<TableEntry>>>;

/// Spawn the refresh loop. Read failures (e.g. the registry dataset not written yet) are
/// logged and skipped rather than propagated — the cache simply stays at its previous
/// value (empty, initially) until a read succeeds.
pub fn spawn_refresh(
    registry_path: String,
    cache: RegistryCache,
    interval: Duration,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            match catalog_core::read_registry(&registry_path).await {
                Ok(Some(entries)) => {
                    *cache.write().await = entries;
                }
                // Registry not written yet (before the first sweep): keep the current (empty)
                // cache rather than clobbering it.
                Ok(None) => {}
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
        let handle = spawn_refresh(path.clone(), cache.clone(), Duration::from_millis(20));

        // Nothing written yet: cache stays empty.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(cache.read().await.is_empty());

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
