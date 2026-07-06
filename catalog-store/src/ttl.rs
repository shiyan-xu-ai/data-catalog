//! TTL hard-delete is not implemented. This module previously removed an entire object-store
//! prefix tree (a table version's `<table>/<timestamp>/` dir); that logic has been removed so
//! the backend has no code path capable of deleting cataloged table data from S3. `delete_prefix`
//! is kept as a stub, unused by production code, marking where real deletion would plug back in.

use anyhow::{bail, Result};
use object_store::path::Path as ObjPath;
use object_store::ObjectStore;

/// Not implemented. Always returns an error without listing or touching `store`, so a caller
/// can never mistake this for a successful no-op.
pub async fn delete_prefix(_store: &dyn ObjectStore, _prefix: &ObjPath) -> Result<()> {
    bail!("delete_prefix is not implemented: this deployment does not delete cataloged data")
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::TryStreamExt;
    use object_store::local::LocalFileSystem;
    use object_store::{ObjectStoreExt, PutPayload};

    #[tokio::test]
    async fn delete_prefix_errors_and_leaves_every_object_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let store = LocalFileSystem::new_with_prefix(dir.path()).unwrap();
        store
            .put(
                &ObjPath::from("t/2026-01-01/a.txt"),
                PutPayload::from_static(b"a"),
            )
            .await
            .unwrap();

        let result = delete_prefix(&store, &ObjPath::from("t/2026-01-01")).await;
        assert!(result.is_err(), "delete_prefix must not report success");

        let mut listing = store.list(Some(&ObjPath::from("t/2026-01-01")));
        assert!(
            listing.try_next().await.unwrap().is_some(),
            "delete_prefix must not touch the store"
        );
    }
}
