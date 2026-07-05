//! Idempotent reconciliation of freshly-swept table state into the registry.
//!
//! A sweep lists every timestamp directory currently on S3 for a table, so the swept version set
//! is the complete current truth for that table (and the sweep only reconciles tables that swept
//! cleanly). `apply_sweep_result` therefore reconciles the registry to the swept set: versions
//! still on S3 are kept/added, and versions absent from the sweep have been removed from S3 and
//! are dropped from the registry too, so an out-of-band deletion (or a TTL apply) is reflected
//! rather than leaving a phantom entry. Applying the same swept set twice is a no-op.
//! `owner`/`ttl_policy` are API-assigned only — the sweep never touches them, and a version's
//! API-set `protected` flag is carried across re-observation.

use std::collections::HashMap;

use crate::types::{TableEntry, TableVersion};

/// Merge a freshly-swept `TableEntry` into the current registry state, keyed by table id.
pub fn apply_sweep_result(registry: &mut HashMap<String, TableEntry>, swept: TableEntry) {
    match registry.get_mut(&swept.id) {
        None => {
            let mut entry = swept;
            recompute_aux_latest(&mut entry);
            registry.insert(entry.id.clone(), entry);
        }
        Some(existing) => {
            existing.name = swept.name;
            existing.namespace = swept.namespace;
            existing.root_location = swept.root_location;
            existing.last_swept = max_option(existing.last_swept, swept.last_swept);
            merge_versions(&mut existing.versions, swept.versions);
            recompute_aux_latest(existing);
        }
    }
}

/// Reconcile `existing` to the freshly-swept version set `incoming` (the complete current S3
/// truth for the table). Versions in `incoming` are kept/added; versions absent from it were
/// removed from S3 and are dropped. For a version that persists across the sweep the API-set
/// `protected` flag is carried forward, and a previously clean classification is not downgraded
/// if this sweep transiently re-observed the version as `partial` (a momentary open failure must
/// not lose good data); a previously `partial` version is upgraded once a sweep classifies it
/// cleanly, so it doesn't stay permanently partial (and permanently TTL-ineligible).
fn merge_versions(existing: &mut Vec<TableVersion>, incoming: Vec<TableVersion>) {
    let reconciled = incoming
        .into_iter()
        .map(
            |v| match existing.iter().find(|e| e.version_id == v.version_id) {
                // Keep the previously-clean version if this sweep re-observed it as partial.
                // (`prev` already carries the API-set `protected` flag.)
                Some(prev) if !prev.partial && v.partial => prev.clone(),
                // Otherwise take the freshly-swept version, preserving `protected`.
                Some(prev) => TableVersion {
                    protected: prev.protected,
                    ..v
                },
                None => v,
            },
        )
        .collect();
    *existing = reconciled;
}

/// Recompute `aux_latest` from the version with the lexicographically-greatest `version_id`
/// (ISO8601 timestamps sort chronologically as strings). Deriving it from merged state rather
/// than carrying it through per-call keeps the result order-independent.
///
/// The assignment is total: when `versions` is empty, `aux_latest` is cleared. This matters
/// after a TTL apply removes the latest (or all) versions -- otherwise `aux_latest` would keep
/// pointing at aux paths that were just hard-deleted from S3. Exposed (and re-exported from the
/// crate root) so `catalog-api`'s TTL apply can restore the invariant after removing versions.
pub fn recompute_aux_latest(entry: &mut TableEntry) {
    entry.aux_latest = entry
        .versions
        .iter()
        .max_by(|a, b| a.version_id.cmp(&b.version_id))
        .map(|latest| latest.aux.clone())
        .unwrap_or_default();
}

fn max_option<T: Ord>(a: Option<T>, b: Option<T>) -> Option<T> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AuxEntry, AuxFormat, Namespace, VersionShape};
    use chrono::{TimeZone, Utc};

    fn version(id: &str, partial: bool, shape: VersionShape) -> TableVersion {
        let ts = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        TableVersion {
            version_id: id.to_string(),
            timestamp: ts,
            snapshot_path: format!("s3://bucket/table/{id}"),
            shape,
            partial,
            protected: false,
            storage_bytes_total: 100,
            lance_core_bytes: 60,
            sidecar_bytes: 0,
            segments_bytes: 40,
            other_aux_bytes: 0,
            row_count: Some(10),
            num_fragments: Some(1),
            schema_json: None,
            num_indices: Some(0),
            aux: vec![AuxEntry {
                name: "segments".to_string(),
                path: format!("s3://bucket/table/{id}/segments"),
                format: AuxFormat::Parquet,
                role: "segments".to_string(),
                storage_bytes: 40,
                fingerprint: None,
            }],
            swept_at: ts,
        }
    }

    fn table_with_versions(versions: Vec<TableVersion>) -> TableEntry {
        TableEntry {
            id: "t1".to_string(),
            name: "t1".to_string(),
            namespace: Namespace::new(["scenario_dataset_export"]),
            root_location: "s3://bucket/table".to_string(),
            owner: None,
            ttl_policy: None,
            last_swept: Some(Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()),
            versions,
            aux_latest: vec![],
        }
    }

    #[test]
    fn duplicate_version_applied_twice_is_a_no_op() {
        let mut registry = HashMap::new();
        let swept = table_with_versions(vec![version(
            "2026-01-01T00-00-00",
            false,
            VersionShape::Full,
        )]);

        apply_sweep_result(&mut registry, swept.clone());
        let after_first = registry.get("t1").cloned().unwrap();

        apply_sweep_result(&mut registry, swept);
        let after_second = registry.get("t1").cloned().unwrap();

        assert_eq!(after_first, after_second);
        assert_eq!(after_second.versions.len(), 1);
    }

    /// Sorted version ids currently recorded for table `t1`.
    fn version_ids(registry: &HashMap<String, TableEntry>) -> Vec<String> {
        let mut ids: Vec<String> = registry
            .get("t1")
            .unwrap()
            .versions
            .iter()
            .map(|v| v.version_id.clone())
            .collect();
        ids.sort();
        ids
    }

    #[test]
    fn reconciles_to_the_swept_set_adding_and_removing_versions() {
        let mut registry = HashMap::new();
        let v1 = version("2026-01-01T00-00-00", false, VersionShape::Full);
        let v2 = version("2026-01-02T00-00-00", false, VersionShape::Full);
        let v3 = version("2026-01-03T00-00-00", false, VersionShape::Full);

        // First sweep observes v1 + v2.
        apply_sweep_result(&mut registry, table_with_versions(vec![v1.clone(), v2]));
        assert_eq!(
            version_ids(&registry),
            vec!["2026-01-01T00-00-00", "2026-01-02T00-00-00"]
        );

        // Next sweep observes only v1 + v3: v2 was removed from S3 (dropped) and v3 is new
        // (added). The registry reconciles to the swept set, and aux_latest tracks the new
        // chronologically-latest version.
        let v3_aux = v3.aux.clone();
        apply_sweep_result(&mut registry, table_with_versions(vec![v1, v3]));
        assert_eq!(
            version_ids(&registry),
            vec!["2026-01-01T00-00-00", "2026-01-03T00-00-00"]
        );
        assert_eq!(registry.get("t1").unwrap().aux_latest, v3_aux);
    }

    #[test]
    fn partial_upgrades_to_clean_but_clean_never_downgrades_and_protected_survives() {
        let mut registry = HashMap::new();

        // First sweep observes the version as partial (e.g. its dataset failed to open), and it
        // is protected via the API.
        let mut partial = version("2026-01-01T00-00-00", true, VersionShape::LanceOnlyPartial);
        partial.protected = true;
        apply_sweep_result(&mut registry, table_with_versions(vec![partial]));

        // Later sweep classifies the same version cleanly: it must upgrade, keeping `protected`.
        let clean = version("2026-01-01T00-00-00", false, VersionShape::Full);
        apply_sweep_result(&mut registry, table_with_versions(vec![clean]));
        let v = &registry.get("t1").unwrap().versions[0];
        assert_eq!(
            v.shape,
            VersionShape::Full,
            "partial must upgrade to clean classification"
        );
        assert!(!v.partial);
        assert!(
            v.protected,
            "API-set protected flag must survive the upgrade"
        );

        // A later sweep that transiently re-observes it as partial must NOT downgrade the clean
        // version back to partial.
        let regressed = version("2026-01-01T00-00-00", true, VersionShape::LanceOnlyPartial);
        apply_sweep_result(&mut registry, table_with_versions(vec![regressed]));
        let v = &registry.get("t1").unwrap().versions[0];
        assert_eq!(
            v.shape,
            VersionShape::Full,
            "a clean version must never be downgraded"
        );
        assert!(!v.partial);
    }
}
