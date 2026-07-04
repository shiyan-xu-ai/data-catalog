//! TTL eligibility computation: which versions of a table are eligible for hard-delete under
//! its per-table `TtlPolicy`. See findings.md's "TTL -- HARD DELETE, PER-TABLE API POLICY"
//! (locked 2026-07-03) for the exact semantics this implements.

use std::collections::HashSet;

use chrono::{DateTime, Utc};

use crate::types::{TableVersion, TtlPolicy, VersionShape};

/// Shapes TTL is willing to delete. `LanceOnlyPartial` and `Empty` are refused outright
/// (safety gate, findings.md "TTL hard-delete safety": we can't be confident about what
/// exactly we'd be deleting for a version whose shape wasn't cleanly classified).
fn is_deletable_shape(shape: VersionShape) -> bool {
    matches!(
        shape,
        VersionShape::Full | VersionShape::LanceOnly | VersionShape::SegOnly
    )
}

/// Compute the TTL-eligible versions of `versions` under `policy`, as of `now`.
///
/// A version is eligible only if ALL of the following hold:
/// - it is not `protected` (never eligible, regardless of policy or shape);
/// - its shape passes the safety gate (`is_deletable_shape`);
/// - it fails every threshold check that is actually SET on `policy`. An unset threshold
///   does not constrain -- it simply isn't one of the checks that must be failed. If NEITHER
///   `keep_last_n` nor `max_age_days` is set, nothing is ever eligible (no policy = no TTL).
///
/// `keep_last_n` ranks versions by `timestamp` descending (most recent first); a version is
/// "beyond" the keep count if it does not fall in the top `keep_last_n` most recent. Ties in
/// `timestamp` are broken arbitrarily but consistently within one call (stable sort).
/// `max_age_days` compares `now - timestamp` against the threshold.
pub fn ttl_eligible_versions<'a>(
    policy: &TtlPolicy,
    versions: &'a [TableVersion],
    now: DateTime<Utc>,
) -> Vec<&'a TableVersion> {
    if policy.keep_last_n.is_none() && policy.max_age_days.is_none() {
        return Vec::new();
    }

    let kept_by_recency: HashSet<&str> = match policy.keep_last_n {
        Some(n) => {
            let mut by_recency: Vec<&TableVersion> = versions.iter().collect();
            by_recency.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
            by_recency
                .into_iter()
                .take(n as usize)
                .map(|v| v.version_id.as_str())
                .collect()
        }
        None => HashSet::new(),
    };

    versions
        .iter()
        .filter(|v| !v.protected)
        .filter(|v| is_deletable_shape(v.shape))
        .filter(|v| {
            let beyond_keep_count =
                policy.keep_last_n.is_none() || !kept_by_recency.contains(v.version_id.as_str());
            let older_than_max_age = policy.max_age_days.is_none()
                || policy.max_age_days.is_some_and(|days| {
                    (now - v.timestamp) > chrono::Duration::days(i64::from(days))
                });
            beyond_keep_count && older_than_max_age
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::AuxEntry;
    use chrono::TimeZone;

    fn version_at(id: &str, days_ago: i64, shape: VersionShape, protected: bool) -> TableVersion {
        let now = Utc.with_ymd_and_hms(2026, 7, 3, 0, 0, 0).unwrap();
        let ts = now - chrono::Duration::days(days_ago);
        TableVersion {
            version_id: id.to_string(),
            timestamp: ts,
            snapshot_path: format!("s3://bucket/table/{id}"),
            shape,
            partial: !matches!(shape, VersionShape::Full),
            protected,
            storage_bytes_total: 1000,
            lance_core_bytes: 1000,
            sidecar_bytes: 0,
            segments_bytes: 0,
            other_aux_bytes: 0,
            row_count: Some(1),
            num_fragments: Some(1),
            schema_json: None,
            num_indices: Some(0),
            aux: Vec::<AuxEntry>::new(),
            swept_at: ts,
        }
    }

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 3, 0, 0, 0).unwrap()
    }

    fn ids(eligible: Vec<&TableVersion>) -> Vec<String> {
        let mut v: Vec<String> = eligible.into_iter().map(|v| v.version_id.clone()).collect();
        v.sort();
        v
    }

    #[test]
    fn neither_threshold_set_means_nothing_is_ever_eligible() {
        let versions = vec![
            version_at("v1", 1000, VersionShape::Full, false),
            version_at("v2", 2000, VersionShape::Full, false),
        ];
        let policy = TtlPolicy::default();
        assert!(ttl_eligible_versions(&policy, &versions, now()).is_empty());
    }

    #[test]
    fn keep_last_n_only_excludes_the_n_most_recent_regardless_of_age() {
        // All versions are ancient, but only keep_last_n constrains: the 2 most recent
        // survive, everything else is eligible.
        let versions = vec![
            version_at("newest", 1, VersionShape::Full, false),
            version_at("middle", 5, VersionShape::Full, false),
            version_at("oldest", 10, VersionShape::Full, false),
        ];
        let policy = TtlPolicy {
            keep_last_n: Some(2),
            max_age_days: None,
        };
        let eligible = ttl_eligible_versions(&policy, &versions, now());
        assert_eq!(ids(eligible), vec!["oldest".to_string()]);
    }

    #[test]
    fn max_age_days_only_excludes_versions_within_the_age_threshold() {
        let versions = vec![
            version_at("young", 1, VersionShape::Full, false),
            version_at("old", 100, VersionShape::Full, false),
        ];
        let policy = TtlPolicy {
            keep_last_n: None,
            max_age_days: Some(30),
        };
        let eligible = ttl_eligible_versions(&policy, &versions, now());
        assert_eq!(ids(eligible), vec!["old".to_string()]);
    }

    #[test]
    fn both_set_requires_exceeding_both_thresholds() {
        // beyond keep-count but young -> not eligible (fails age check).
        // within keep-count but old -> not eligible (fails keep-count check).
        // beyond keep-count AND old -> eligible.
        // Ranked by recency (descending): recent1 (1d, rank0), recent2 (2d, rank1),
        // beyond_but_young (3d, rank2), old_but_kept (100d, rank3).
        let versions = vec![
            version_at("recent1", 1, VersionShape::Full, false),
            version_at("recent2", 2, VersionShape::Full, false),
            version_at("old_but_kept", 100, VersionShape::Full, false),
            version_at("beyond_but_young", 3, VersionShape::Full, false),
        ];
        // keep_last_n=2 keeps ranks 0,1 (recent1, recent2); ranks 2,3 (beyond_but_young,
        // old_but_kept) are both beyond the keep count.
        let policy = TtlPolicy {
            keep_last_n: Some(2),
            max_age_days: Some(30),
        };
        let eligible = ttl_eligible_versions(&policy, &versions, now());
        // old_but_kept: beyond keep-count (rank 2 >= 2) AND older than 30 days -> eligible.
        // beyond_but_young: beyond keep-count AND but only 3 days old -> NOT eligible.
        // recent1/recent2: within keep-count -> NOT eligible regardless of age.
        assert_eq!(ids(eligible), vec!["old_but_kept".to_string()]);
    }

    #[test]
    fn protected_version_is_never_eligible_regardless_of_policy() {
        let versions = vec![version_at("v1", 1000, VersionShape::Full, true)];
        let policy = TtlPolicy {
            keep_last_n: Some(0),
            max_age_days: Some(1),
        };
        assert!(ttl_eligible_versions(&policy, &versions, now()).is_empty());
    }

    #[test]
    fn lance_only_partial_shape_is_excluded_by_the_safety_gate_even_if_otherwise_eligible() {
        let versions = vec![
            version_at("partial_old", 1000, VersionShape::LanceOnlyPartial, false),
            version_at("full_old", 1000, VersionShape::Full, false),
        ];
        let policy = TtlPolicy {
            keep_last_n: Some(0),
            max_age_days: Some(1),
        };
        let eligible = ttl_eligible_versions(&policy, &versions, now());
        assert_eq!(ids(eligible), vec!["full_old".to_string()]);
    }

    #[test]
    fn empty_shape_is_excluded_by_the_safety_gate() {
        let versions = vec![version_at("empty_old", 1000, VersionShape::Empty, false)];
        let policy = TtlPolicy {
            keep_last_n: Some(0),
            max_age_days: Some(1),
        };
        assert!(ttl_eligible_versions(&policy, &versions, now()).is_empty());
    }
}
