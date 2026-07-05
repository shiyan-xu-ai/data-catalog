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

        // Nothing written yet, but a successful read (registry-absent) still marks the pod
        // hydrated. Poll with a deadline rather than a fixed sleep so this is robust under slow
        // CI scheduling (a fixed 50ms wait against the 20ms refresh interval was flaky).
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while !hydrated.load(Ordering::SeqCst) {
            assert!(
                tokio::time::Instant::now() < deadline,
                "refresh loop never completed a read to mark the pod hydrated"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        // Nothing has been written yet, so the cache must still be empty.
        assert!(cache.read().await.is_empty());

        // Write the registry "out of band" (as the leader's sweep loop would), then poll until
        // the refresh loop picks it up.
        catalog_core::write_registry(&path, &[sample_entry("t1")])
            .await
            .unwrap();

        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            {
                let cached = cache.read().await;
                if cached.len() == 1 {
                    assert_eq!(cached[0].id, "t1");
                    break;
                }
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "refresh loop never picked up the out-of-band registry write"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        handle.abort();
    }
}
