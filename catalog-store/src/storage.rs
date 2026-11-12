use std::collections::HashSet;
use std::sync::Arc;

use futures::TryStreamExt;
use lance::index::DatasetIndexExt;
use lance::Dataset;
use lance_index::is_system_index;
use lance_namespace::{Error, Result};
use lance_table::io::deletion::relative_deletion_file_path;

use crate::cache::{ObjectTotals, StorageListing};
use crate::namespace::{lance_error, OpenTable, PostgresNamespace};

/// Object-storage categories of a dataset prefix, in response order.
pub const STORAGE_CATEGORIES: [&str; 7] = [
    "data",
    "blob",
    "index",
    "deletion",
    "manifest",
    "transaction",
    "other",
];

/// Bytes and object count of one storage category.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageCategory {
    pub name: &'static str,
    pub bytes: u64,
    pub objects: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageBreakdown {
    pub total_bytes: u64,
    pub total_objects: u64,
    /// One entry per [`STORAGE_CATEGORIES`] name, in that order.
    pub categories: Vec<StorageCategory>,
}

impl StorageBreakdown {
    fn new(totals: &[ObjectTotals]) -> Self {
        let categories: Vec<StorageCategory> = STORAGE_CATEGORIES
            .iter()
            .zip(totals)
            .map(|(&name, totals)| StorageCategory {
                name,
                bytes: totals.bytes,
                objects: totals.objects,
            })
            .collect();
        Self {
            total_bytes: categories.iter().map(|c| c.bytes).sum(),
            total_objects: categories.iter().map(|c| c.objects).sum(),
            categories,
        }
    }
}

/// A table version's data next to everything stored under its location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageFootprint {
    pub version: u64,
    /// Data files the version references (manifest `total_files_size`).
    pub data_bytes: u64,
    /// Files of the version's indices.
    pub index_bytes: u64,
    /// Objects under the location that the version references.
    pub current: StorageBreakdown,
    /// Every object under the location, all versions included.
    pub all: StorageBreakdown,
}

impl PostgresNamespace {
    /// Lists every object under the registered location (metadata only, never writes) once per
    /// current version and instance, bucketed by path relative to that location, both overall
    /// and for the objects the current version references. The prefix match is per path
    /// segment, so sibling directories such as `<table>.lance_tags/` are excluded.
    pub async fn storage_footprint(&self, id: &[String]) -> Result<StorageFootprint> {
        self.read_table(id, None, |table| async move {
            let location = &table.location;
            let index_bytes = async {
                let descriptions = table
                    .dataset
                    .describe_indices(None)
                    .await
                    .map_err(lance_error(format!("describe indices at {location}")))?;
                Ok::<u64, Error>(
                    descriptions
                        .iter()
                        .filter(|d| d.metadata().first().is_some_and(|m| !is_system_index(m)))
                        .filter_map(|d| d.total_size_bytes())
                        .sum(),
                )
            };
            let (index_bytes, listing) = futures::try_join!(index_bytes, table.storage_listing())?;
            Ok(StorageFootprint {
                version: table.dataset.version_id(),
                data_bytes: table.dataset.manifest().summary().total_files_size,
                index_bytes,
                current: StorageBreakdown::new(&listing.current),
                all: StorageBreakdown::new(&listing.all),
            })
        })
        .await
    }
}

impl OpenTable {
    async fn storage_listing(&self) -> Result<Arc<StorageListing>> {
        self.cached(|dataset, location| async move {
            let store = dataset
                .object_store(None)
                .await
                .map_err(lance_error(format!("open object store at {location}")))?;
            let referenced = ReferencedFiles::of(&dataset)
                .await
                .map_err(lance_error(format!("load indices at {location}")))?;
            let base = dataset.branch_location().path;
            let mut all = vec![ObjectTotals::default(); STORAGE_CATEGORIES.len()];
            let mut current = all.clone();
            let mut objects = store.read_dir_all(&base, None);
            while let Some(meta) = objects
                .try_next()
                .await
                .map_err(lance_error(format!("list objects at {location}")))?
            {
                let relative = relative_to(base.as_ref(), meta.location.as_ref());
                let category = storage_category(relative);
                all[category].add(meta.size);
                if referenced.contains(relative) {
                    current[category].add(meta.size);
                }
            }
            Ok(StorageListing { current, all })
        })
        .await
    }
}

/// `path` relative to the dataset location `base`, or `path` itself when outside it.
pub(crate) fn relative_to<'a>(base: &str, path: &'a str) -> &'a str {
    path.strip_prefix(base)
        .map_or(path, |p| p.trim_start_matches('/'))
}

/// Index into [`STORAGE_CATEGORIES`] for a path relative to the dataset location.
fn storage_category(relative: &str) -> usize {
    let category = match relative.split_once('/') {
        Some(("data", file)) if file.ends_with(".lance") => "data",
        Some(("data", file)) if file.ends_with(".blob") => "blob",
        Some(("_indices", _)) => "index",
        Some(("_deletions", _)) => "deletion",
        Some(("_versions", _)) => "manifest",
        Some(("_transactions", _)) => "transaction",
        _ => "other",
    };
    STORAGE_CATEGORIES
        .iter()
        .position(|&name| name == category)
        .expect("category is listed")
}

struct ReferencedFiles {
    files: HashSet<String>,
    /// `<key>` of each data file `data/<key>.lance`; its sidecar blobs live at
    /// `data/<key>/<id>.blob`.
    data_keys: HashSet<String>,
    indices: HashSet<String>,
}

impl ReferencedFiles {
    async fn of(dataset: &Dataset) -> lance::Result<Self> {
        let manifest = dataset.manifest();
        let mut files = HashSet::new();
        let mut data_keys = HashSet::new();
        for fragment in manifest.fragments.iter() {
            for file in fragment.files.iter().filter(|f| f.base_id.is_none()) {
                files.insert(format!("data/{}", file.path));
                if let Some(key) = file.path.strip_suffix(".lance") {
                    data_keys.insert(key.to_string());
                }
            }
            if let Some(deletion) = &fragment.deletion_file {
                files.insert(relative_deletion_file_path(fragment.id, deletion));
            }
        }
        let base = dataset.branch_location().path;
        files.insert(
            relative_to(base.as_ref(), dataset.manifest_location().path.as_ref()).to_string(),
        );
        if let Some(transaction) = manifest
            .transaction_file
            .as_deref()
            .filter(|t| !t.is_empty())
        {
            files.insert(format!("_transactions/{transaction}"));
        }
        let indices = dataset
            .load_indices()
            .await?
            .iter()
            .map(|index| index.uuid.to_string())
            .collect();
        Ok(Self {
            files,
            data_keys,
            indices,
        })
    }

    fn contains(&self, relative: &str) -> bool {
        if self.files.contains(relative) {
            return true;
        }
        if let Some((key, _)) = relative
            .strip_prefix("data/")
            .and_then(|rest| rest.split_once('/'))
            .filter(|(_, file)| file.ends_with(".blob"))
        {
            return self.data_keys.contains(key);
        }
        relative
            .strip_prefix("_indices/")
            .and_then(|rest| rest.split_once('/'))
            .is_some_and(|(uuid, _)| self.indices.contains(uuid))
    }
}
