//! `catalog-api` library: config, the merged read model, the sweep, and API routing, shared
//! between the `catalog-api` binary and its integration tests.

pub mod api;
pub mod catalog;
pub mod config;
pub mod secrets;
pub mod shutdown;
pub mod sweep;
pub mod sweep_config;
pub mod webui;
