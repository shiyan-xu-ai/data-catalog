//! Object-store IO and S3 sync logic for the data catalog.

mod config;
mod format;
mod overlay;
mod sync;

pub use config::{DeepStats, SyncConfig, DEFAULT_SYNC_CONCURRENCY};
pub use overlay::{MetaStore, TableMeta};
pub use sync::{
    deep_for, list_version_dirs, sync_root, sync_table, sync_version, SyncOutcome, SyncedVersion,
    VersionDirRef,
};
