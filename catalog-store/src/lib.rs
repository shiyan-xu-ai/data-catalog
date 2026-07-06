//! Object-store IO and S3 sync logic for the data catalog.

mod config;
mod format;
mod overlay;
mod sync;
mod ttl;

pub use config::{DeepStats, SyncConfig, DEFAULT_SYNC_CONCURRENCY};
pub use format::recursive_bytes;
pub use overlay::{MetaStore, TableMeta};
pub use sync::{
    deep_for, list_version_dirs, sync_root, sync_table, sync_version, SyncOutcome, SyncedVersion,
    VersionDirRef,
};
pub use ttl::delete_prefix;
