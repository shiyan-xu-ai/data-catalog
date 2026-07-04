//! Idempotent merge of freshly-swept table state into the registry.
//!
//! Timestamp-path versions are immutable snapshots: once a `version_id` is recorded it is
//! never overwritten. Applying the same sweep result twice, or applying results for several
//! versions in any order, converges to the same final state (union of all versions seen,
//! deduped by `version_id`). `owner`/`ttl_policy` are API-assigned only — sweep never touches
//! them.

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

/// Union incoming versions into `existing`, deduped by `version_id`.
///
/// A cleanly-classified version is an immutable snapshot: once recorded it is never overwritten,
/// and a later sweep that transiently fails to classify it (a `partial` re-observation) must
/// NOT downgrade it. The one exception is upgrading a previously-`partial` version: if an
/// earlier sweep recorded it as partial (e.g. its dataset failed to open) and a later sweep
/// classifies it cleanly, replace it so it stops being permanently stuck partial (and therefore
/// permanently TTL-ineligible). The API-set `protected` flag is preserved across that upgrade.
fn merge_versions(existing: &mut Vec<TableVersion>, incoming: Vec<TableVersion>) {
    for v in incoming {
        match existing.iter_mut().find(|e| e.version_id == v.version_id) {
            None => existing.push(v),
            Some(existing_v) if existing_v.partial && !v.partial => {
                let was_protected = existing_v.protected;
                *existing_v = v;
                existing_v.protected = was_protected;
            }
            Some(_) => {}
        }
    }
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

    #[test]
    fn versions_applied_out_of_order_converge_to_same_state() {
        let v1 = version("2026-01-01T00-00-00", false, VersionShape::Full);
        let v2 = version("2026-01-02T00-00-00", false, VersionShape::Full);

        let mut in_order = HashMap::new();
        apply_sweep_result(&mut in_order, table_with_versions(vec![v1.clone()]));
        apply_sweep_result(&mut in_order, table_with_versions(vec![v2.clone()]));

        let mut out_of_order = HashMap::new();
        apply_sweep_result(&mut out_of_order, table_with_versions(vec![v2.clone()]));
        apply_sweep_result(&mut out_of_order, table_with_versions(vec![v1.clone()]));

        let mut in_order_entry = in_order.remove("t1").unwrap();
        let mut out_of_order_entry = out_of_order.remove("t1").unwrap();
        in_order_entry
            .versions
            .sort_by(|a, b| a.version_id.cmp(&b.version_id));
        out_of_order_entry
            .versions
            .sort_by(|a, b| a.version_id.cmp(&b.version_id));

        assert_eq!(in_order_entry, out_of_order_entry);
        assert_eq!(in_order_entry.versions.len(), 2);
        // aux_latest derives from the chronologically-latest version regardless of apply order.
        assert_eq!(in_order_entry.aux_latest, v2.aux);
    }

    #[test]
    fn partial_shape_is_not_silently_dropped_when_merged_with_existing_state() {
        let mut registry = HashMap::new();
        apply_sweep_result(
            &mut registry,
            table_with_versions(vec![version(
                "2026-01-01T00-00-00",
                false,
                VersionShape::Full,
            )]),
        );

        let partial = version("2026-01-02T00-00-00", true, VersionShape::SegOnly);
        apply_sweep_result(&mut registry, table_with_versions(vec![partial.clone()]));

        let entry = registry.get("t1").unwrap();
        let stored = entry
            .versions
            .iter()
            .find(|v| v.version_id == "2026-01-02T00-00-00")
            .unwrap();
        assert!(stored.partial);
        assert_eq!(stored.shape, VersionShape::SegOnly);

        // Re-applying the same partial version must not flip it back or drop it.
        apply_sweep_result(&mut registry, table_with_versions(vec![partial]));
        let entry = registry.get("t1").unwrap();
        assert_eq!(entry.versions.len(), 2);
        let stored = entry
            .versions
            .iter()
            .find(|v| v.version_id == "2026-01-02T00-00-00")
            .unwrap();
        assert!(stored.partial);
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
