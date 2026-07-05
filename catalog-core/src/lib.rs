//! Core types for the data catalog registry: table/version/namespace/TTL models, the
//! registry Lance table reader/writer, and the `aux_latest` derivation.

mod apply;
mod registry;
mod ttl;
mod ttl_audit;
mod types;

pub use apply::recompute_aux_latest;
pub use registry::{cleanup_registry, read_registry, write_registry};
pub use ttl::ttl_eligible_versions;
pub use ttl_audit::{append_ttl_audit, read_ttl_audit};
pub use types::{
    AuxEntry, AuxFormat, Namespace, TableEntry, TableVersion, TtlAuditRecord, TtlPolicy,
    VersionShape,
};
