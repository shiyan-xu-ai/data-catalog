//! TTL hard-delete: physically remove an entire object-store prefix tree (a table version's
//! `<table>/<timestamp>/` dir, main lance dataset + all aux). No-op (not an error) if nothing
//! exists under the prefix, so re-applying TTL against an already-deleted version is
//! idempotent.

use anyhow::Result;
use futures::TryStreamExt;
use object_store::path::Path as ObjPath;
use object_store::{ObjectStore, ObjectStoreExt};

/// Recursively delete every object under `prefix`. LIST-then-delete-each rather than a bulk
/// API: `object_store::ObjectStore` exposes `delete_stream` for a caller-supplied stream of
/// paths, but not a single "delete everything under this prefix" call, so we materialize the
/// listing first (bounded by one version's object count, not the whole table).
pub async fn delete_prefix(store: &dyn ObjectStore, prefix: &ObjPath) -> Result<()> {
    let mut stream = store.list(Some(prefix));
    let mut paths = Vec::new();
    while let Some(meta) = stream.try_next().await? {
        paths.push(meta.location);
    }
    for path in paths {
        match store.delete(&path).await {
            Ok(()) => {}
            // Idempotent re-apply: another delete (or a prior partially-applied call) may
            // already have removed this object.
            Err(object_store::Error::NotFound { .. }) => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use object_store::local::LocalFileSystem;
    use object_store::{ObjectStoreExt, PutPayload};

    #[tokio::test]
    async fn delete_prefix_removes_everything_under_it_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let store = LocalFileSystem::new_with_prefix(dir.path()).unwrap();
        store
            .put(
                &ObjPath::from("t/2026-01-01/a.txt"),
                PutPayload::from_static(b"a"),
            )
            .await
            .unwrap();
        store
            .put(
                &ObjPath::from("t/2026-01-01/sub/b.txt"),
                PutPayload::from_static(b"b"),
            )
            .await
            .unwrap();
        store
            .put(
                &ObjPath::from("t/other/c.txt"),
                PutPayload::from_static(b"c"),
            )
            .await
            .unwrap();

        delete_prefix(&store, &ObjPath::from("t/2026-01-01"))
            .await
            .unwrap();

        let mut deleted = store.list(Some(&ObjPath::from("t/2026-01-01")));
        assert!(
            deleted.try_next().await.unwrap().is_none(),
            "prefix should be fully empty after delete"
        );
        let mut sibling = store.list(Some(&ObjPath::from("t/other")));
        assert!(
            sibling.try_next().await.unwrap().is_some(),
            "sibling prefix must be untouched"
        );
    }

    #[tokio::test]
    async fn delete_prefix_on_an_already_empty_prefix_is_a_no_op_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let store = LocalFileSystem::new_with_prefix(dir.path()).unwrap();
        delete_prefix(&store, &ObjPath::from("nothing/here"))
            .await
            .expect("deleting an already-empty/nonexistent prefix must not error");
    }
}
