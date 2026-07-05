//! Derivation of a table's `aux_latest` from its version set.
//!
//! The sweep re-derives each registered table's version set whole from immutable S3 content, so
//! there is no cross-sweep merge to perform — the swept result IS the derived snapshot. The one
//! derived field not read straight off a single version is `aux_latest` (the aux of the
//! chronologically-latest version), which `recompute_aux_latest` computes from the version set.

use crate::types::TableEntry;

/// Recompute `aux_latest` from the version with the lexicographically-greatest `version_id`
/// (ISO8601 timestamps sort chronologically as strings). Deriving it from the version set rather
/// than carrying it through keeps the result order-independent.
///
/// The assignment is total: when `versions` is empty, `aux_latest` is cleared. This matters
/// after a TTL apply removes the latest (or all) versions -- otherwise `aux_latest` would keep
/// pointing at aux paths that were just hard-deleted from S3. Used by the read-model merge and by
/// TTL apply to restore the invariant after removing versions.
pub fn recompute_aux_latest(entry: &mut TableEntry) {
    entry.aux_latest = entry
        .versions
        .iter()
        .max_by(|a, b| a.version_id.cmp(&b.version_id))
        .map(|latest| latest.aux.clone())
        .unwrap_or_default();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AuxEntry, AuxFormat, Namespace, TableVersion, VersionShape};
    use chrono::{TimeZone, Utc};

    fn version(id: &str) -> TableVersion {
        let ts = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        TableVersion {
            version_id: id.to_string(),
            timestamp: ts,
            snapshot_path: format!("s3://bucket/table/{id}"),
            shape: VersionShape::Full,
            partial: false,
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

    fn entry(versions: Vec<TableVersion>) -> TableEntry {
        TableEntry {
            id: "t1".to_string(),
            name: "t1".to_string(),
            namespace: Namespace::new(["ns"]),
            root_location: "s3://bucket/table".to_string(),
            owner: None,
            ttl_policy: None,
            last_swept: None,
            versions,
            aux_latest: vec![],
        }
    }

    #[test]
    fn aux_latest_tracks_the_chronologically_latest_version_and_clears_when_empty() {
        // Picks the latest by version_id regardless of input order.
        let v_late = version("2026-01-03T00-00-00");
        let expected = v_late.aux.clone();
        let mut e = entry(vec![v_late, version("2026-01-01T00-00-00")]);
        recompute_aux_latest(&mut e);
        assert_eq!(e.aux_latest, expected);

        // Empty version set clears aux_latest (e.g. after a TTL apply removes every version).
        e.versions.clear();
        recompute_aux_latest(&mut e);
        assert!(e.aux_latest.is_empty());
    }
}
