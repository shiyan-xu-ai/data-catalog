//! Core table/version/namespace types and the registry Lance table reader/writer.

mod registry;
mod table_id;
mod types;

pub use registry::{cleanup_registry, read_registry, write_registry};
pub use table_id::{compose_table_id, parse_table_id, valid_id_segment, ParsedTableId, ID_SEP};
pub use types::{AuxEntry, AuxFormat, Namespace, TableEntry, TableVersion, VersionShape};
