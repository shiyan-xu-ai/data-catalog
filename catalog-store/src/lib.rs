//! Object-store IO and S3 sweep logic for the data catalog.

mod config;
mod format;
mod overlay;
mod sweep;
mod ttl;

pub use config::SweepConfig;
pub use format::{detect_format, recursive_bytes};
pub use overlay::{MetaStore, TableMeta};
pub use sweep::{sweep_root, sweep_table, sweep_version, SweepOutcome};
pub use ttl::delete_prefix;
