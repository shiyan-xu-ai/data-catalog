//! Per-version values cached in the Lance session's metadata cache.
//!
//! A Lance version never changes once written, so everything derived from one version is cached
//! under its location, version number and manifest e-tag. The e-tag keeps a dataset recreated at
//! the same location from reusing an earlier incarnation's entries. Entries are weighed by their
//! estimated size and evicted under the session's budget.

use std::borrow::Cow;
use std::future::Future;
use std::marker::PhantomData;
use std::sync::Arc;

use lance::cache::{CacheKey, LanceCache};
use lance::session::Session;
use lance::Dataset;
use lance_core::deepsize::{Context, DeepSizeOf};
use lance_namespace::Result;

/// A value cached for one table version; `NAME` must be unique per type.
pub(crate) trait VersionValue: DeepSizeOf + Send + Sync + 'static {
    const NAME: &'static str;
}

/// Fixed weight added to an opened dataset beyond its manifest, for its handles and caches.
const DATASET_OVERHEAD_BYTES: usize = 16 * 1024;

/// An opened dataset, weighed by its manifest.
pub(crate) struct OpenedDataset(pub Arc<Dataset>);

impl DeepSizeOf for OpenedDataset {
    fn deep_size_of_children(&self, context: &mut Context) -> usize {
        self.0.manifest().deep_size_of_children(context) + DATASET_OVERHEAD_BYTES
    }
}

impl VersionValue for OpenedDataset {
    const NAME: &'static str = "catalog.OpenedDataset";
}

/// Bytes on disk per field id.
#[derive(lance_core::deepsize::DeepSizeOf)]
pub(crate) struct FieldBytes(pub Vec<(u32, u64)>);

impl VersionValue for FieldBytes {
    const NAME: &'static str = "catalog.FieldBytes";
}

/// Physical rows of every fragment, sorted ascending.
#[derive(lance_core::deepsize::DeepSizeOf)]
pub(crate) struct FragmentRows(pub Vec<u64>);

impl VersionValue for FragmentRows {
    const NAME: &'static str = "catalog.FragmentRows";
}

/// Deleted rows across all fragments.
#[derive(lance_core::deepsize::DeepSizeOf)]
pub(crate) struct DeletedRows(pub u64);

impl VersionValue for DeletedRows {
    const NAME: &'static str = "catalog.DeletedRows";
}

/// `(bytes, objects)` per storage category, in `STORAGE_CATEGORIES` order.
#[derive(lance_core::deepsize::DeepSizeOf)]
pub(crate) struct StorageListing(pub Vec<(u64, u64)>);

impl VersionValue for StorageListing {
    const NAME: &'static str = "catalog.StorageListing";
}

struct VersionKey<'a, V> {
    version: u64,
    e_tag: Option<&'a str>,
    value: PhantomData<V>,
}

impl<V: VersionValue> CacheKey for VersionKey<'_, V> {
    type ValueType = V;

    fn key(&self) -> Cow<'_, str> {
        match self.e_tag {
            Some(e_tag) => Cow::Owned(format!("{}/{e_tag}", self.version)),
            None => Cow::Owned(self.version.to_string()),
        }
    }

    fn type_name() -> &'static str {
        V::NAME
    }
}

/// The cache entries of one dataset location.
pub(crate) struct LocationCache(LanceCache);

impl LocationCache {
    pub(crate) fn new(session: &Session, location: &str) -> Self {
        Self(
            session
                .file_metadata_cache()
                .with_key_prefix("data-catalog")
                .with_key_prefix(location),
        )
    }

    pub(crate) async fn get<V: VersionValue>(
        &self,
        version: u64,
        e_tag: Option<&str>,
    ) -> Option<Arc<V>> {
        self.0.get_with_key(&key::<V>(version, e_tag)).await
    }

    pub(crate) async fn insert<V: VersionValue>(
        &self,
        version: u64,
        e_tag: Option<&str>,
        value: V,
    ) -> Arc<V> {
        let value = Arc::new(value);
        self.0
            .insert_with_key(&key::<V>(version, e_tag), value.clone())
            .await;
        value
    }

    /// The cached value, or `load`'s result cached; concurrent loads of one key share one call.
    pub(crate) async fn get_or_load<V, F, Fut>(
        &self,
        version: u64,
        e_tag: Option<&str>,
        load: F,
    ) -> Result<Arc<V>>
    where
        V: VersionValue,
        F: FnOnce() -> Fut + Send,
        Fut: Future<Output = Result<V>> + Send,
    {
        self.0
            .get_or_insert_with_key(key::<V>(version, e_tag), load)
            .await
    }
}

fn key<V>(version: u64, e_tag: Option<&str>) -> VersionKey<'_, V> {
    VersionKey {
        version,
        e_tag,
        value: PhantomData,
    }
}
