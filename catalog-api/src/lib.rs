//! `catalog-api` library: config, the merged read model, the sync, and API routing, shared
//! between the `catalog-api` binary and its integration tests.

pub mod api;
pub mod catalog;
pub mod catalog_config;
pub mod config;
pub mod identity;
pub mod sample;
pub mod secrets;
pub mod shutdown;
pub mod sync;
pub mod sync_config;
pub mod webui;
