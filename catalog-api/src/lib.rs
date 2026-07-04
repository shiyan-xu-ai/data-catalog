//! `catalog-api` library: config, leader election, sweep loop, and registry-cache modules,
//! shared between the `catalog-api` binary and its integration tests.

pub mod api;
pub mod config;
pub mod internal_server;
pub mod leader;
pub mod metrics;
pub mod registry_cache;
pub mod registry_lock;
pub mod sweep_config;
pub mod sweep_loop;
