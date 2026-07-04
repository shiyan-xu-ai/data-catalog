//! Shared registry write lock.
//!
//! The SAME `Arc<tokio::sync::Mutex<()>>` is threaded through every writer of
//! `_catalog/registry`: the REST API mutation handlers (`DeclareTable`, `DeregisterTable`,
//! version `protect`, TTL `apply`) AND the sweep loop's own read-modify-write. Holding this
//! lock across the whole "read registry -> mutate in-memory -> write registry" critical
//! section means those writers can never interleave.
//!
//! Before this (Phase 5), `ApiState::write_lock` only serialized API mutations against each
//! other; the sweep loop held no lock at all. A sweep could read a stale registry, then write
//! its own (stale) copy back AFTER a concurrent API mutation had already committed a change,
//! silently reverting it -- a genuine lost-update race confirmed by the Phase 5 reviewer.
//! Phase 6 wires TTL hard-delete to the per-version `protected` flag, which makes a
//! silently-reverted `protect` call irreversible (TTL could then delete a version an admin
//! believed was protected) -- so this lock now closes that gap for both writers on the same
//! leader pod. (The residual cross-pod leadership-flip TOCTOU is a separate, already-deferred,
//! bounded/self-correcting hazard -- see status.md's Open items; not addressed here.)

use std::sync::Arc;
use tokio::sync::Mutex;

/// Guards the entire read-registry -> mutate -> write-registry critical section for every
/// writer of `_catalog/registry`.
pub type RegistryWriteLock = Arc<Mutex<()>>;

pub fn new_registry_write_lock() -> RegistryWriteLock {
    Arc::new(Mutex::new(()))
}
