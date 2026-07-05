//! Object-store IO and S3 sweep logic for the data catalog.

mod config;
mod format;
mod overlay;
mod sweep;
mod ttl;

pub use config::{DeepStats, SweepConfig, DEFAULT_SWEEP_CONCURRENCY};
pub use format::recursive_bytes;
pub use overlay::{MetaStore, TableMeta};
pub use sweep::{
    deep_for, list_version_dirs, sweep_root, sweep_table, sweep_version, SweepOutcome,
    SweptVersion, VersionDirRef,
};
pub use ttl::delete_prefix;
