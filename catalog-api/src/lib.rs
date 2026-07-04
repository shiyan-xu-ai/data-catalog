//! `catalog-api` library: config, leader election, sweep loop, and registry-cache modules,
//! shared between the `catalog-api` binary and its integration tests.

pub mod config;
pub mod leader;
pub mod registry_cache;
pub mod sweep_config;
pub mod sweep_loop;
