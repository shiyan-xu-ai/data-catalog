//! Core registry types: tables, versions, namespaces, aux entries, TTL policy/audit.
//!
//! ## Persistence forward/backward compatibility
//!
//! Several of these types are persisted as JSON (`registry`'s `versions_json`/`aux_latest_json`
//! columns, the `ttl_audit` table). During a rolling upgrade two binary versions read each
//! other's rows, so the JSON schema must evolve without breaking either direction. serde already
//! ignores unknown fields on deserialize (new field written, old binary reads it), and every
//! non-identity field below carries `#[serde(default)]` so a field this binary knows but an older
//! row omits deserializes to its default instead of failing the whole read (which, with the
//! registry read path, would otherwise surface as an error).
//!
//! **Policy:** any new persisted field MUST be added with `#[serde(default)]` (and a `Default`-
//! able type). Identity/structural fields (ids, timestamps, the shape/format enums) stay required.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A namespace is a sequence of path segments, e.g. `["scenario_dataset_export"]`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Namespace(pub Vec<String>);

impl Namespace {
    pub fn new(segments: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self(segments.into_iter().map(Into::into).collect())
    }

    pub fn segments(&self) -> &[String] {
        &self.0
    }
}

/// Format of an auxiliary directory attached to a table version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuxFormat {
    Parquet,
    Lance,
    Csv,
    Mixed,
    Unknown,
}

/// One auxiliary directory observed alongside a table version's main lance dataset
/// (e.g. `segments/`, `dataset.sidecar/`, `curated_csv/`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuxEntry {
    /// Directory name as observed on disk (convention-based role; no OpenLineage in v1.0.0).
    pub name: String,
    #[serde(default)]
    pub path: String,
    pub format: AuxFormat,
    /// Convention-derived role (defaults to `name`).
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub storage_bytes: u64,
    /// Cheap fingerprint (e.g. LIST-derived hash) for mixed/unknown formats.
    #[serde(default)]
    pub fingerprint: Option<String>,
    /// Placement taxonomy: `sidecar` (top-level aux: `dataset.sidecar/`, `segments/`, known
    /// top dirs) or `nested_sidecar` (aux living inside the main lance dir).
    #[serde(default)]
    pub category: Option<String>,
    /// For lance-format aux: the openable dataset root, which may sit deeper than `path`
    /// (nested bundles). Sampling/query targets this.
    #[serde(default)]
    pub dataset_path: Option<String>,
    /// For lance-format aux (manifest-derived, deep-stats-gated like the main dataset).
    #[serde(default)]
    pub row_count: Option<u64>,
    #[serde(default)]
    pub schema_json: Option<String>,
    #[serde(default)]
    pub lance_version: Option<u64>,
    #[serde(default)]
    pub writer_version: Option<String>,
}

/// Shape of a synced table version, per findings.md.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionShape {
    /// Main lance dataset (openable) + segments [+ sidecar].
    Full,
    /// Main lance dataset present, no segments.
    LanceOnly,
    /// Segments only, no lance (partial).
    SegOnly,
    /// Main lance dir has only nested lances, no openable root dataset (partial).
    LanceOnlyPartial,
    /// No recognized dirs.
    Empty,
}

/// One timestamp-path snapshot of a table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableVersion {
    /// ISO8601 string of the parsed timestamp path; stable identifier for this version.
    pub version_id: String,
    pub timestamp: DateTime<Utc>,
    pub snapshot_path: String,
    pub shape: VersionShape,
    /// True when this version could not be fully classified/opened (see `shape`).
    #[serde(default)]
    pub partial: bool,
    /// True when the version is exempt from TTL.
    #[serde(default)]
    pub protected: bool,
    #[serde(default)]
    pub storage_bytes_total: u64,
    /// Objects under this version's prefix, from the sync LIST.
    #[serde(default)]
    pub object_count: Option<u64>,
    #[serde(default)]
    pub lance_core_bytes: u64,
    #[serde(default)]
    pub sidecar_bytes: u64,
    #[serde(default)]
    pub segments_bytes: u64,
    #[serde(default)]
    pub other_aux_bytes: u64,
    #[serde(default)]
    pub row_count: Option<u64>,
    #[serde(default)]
    pub num_fragments: Option<u64>,
    #[serde(default)]
    pub schema_json: Option<String>,
    #[serde(default)]
    pub num_indices: Option<u64>,
    /// Lance-internal manifest version of the opened dataset (from its manifest; no extra IO).
    #[serde(default)]
    pub lance_version: Option<u64>,
    /// Writer that produced the dataset, as `<library>/<version>` (from its manifest).
    #[serde(default)]
    pub writer_version: Option<String>,
    #[serde(default)]
    pub aux: Vec<AuxEntry>,
    pub synced_at: DateTime<Utc>,
}

/// A table's registry entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableEntry {
    pub id: String,
    pub name: String,
    /// Deployment region this table's bucket lives in (first id segment).
    #[serde(default)]
    pub region: String,
    /// Target bucket holding the table (second id segment).
    #[serde(default)]
    pub bucket: String,
    pub namespace: Namespace,
    pub root_location: String,
    pub owner: Option<String>,
    pub ttl_policy: Option<TtlPolicy>,
    pub last_synced: Option<DateTime<Utc>>,
    pub versions: Vec<TableVersion>,
    /// Latest version's aux summary, kept denormalized for cheap listing.
    pub aux_latest: Vec<AuxEntry>,
}

/// Per-table TTL policy. A version is TTL-eligible only if it exceeds BOTH configured
/// thresholds (kept if within either); no fields set means no TTL applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TtlPolicy {
    #[serde(default)]
    pub keep_last_n: Option<u32>,
    #[serde(default)]
    pub max_age_days: Option<u32>,
}

/// One row of a storage scan: a bucket prefix (either a registered namespace or an unexplored
/// top-level/sibling/ancestor dir) with its aggregated size when registered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoragePrefixStat {
    pub region: String,
    pub bucket: String,
    /// `/`-joined display path, e.g. `"a"`, `"a/c"`, or the `"(root)"` pseudo-prefix for loose
    /// objects sitting directly at the bucket root.
    pub prefix: String,
    /// True iff `prefix` IS a registered namespace (not an ancestor or sibling of one).
    pub registered: bool,
    #[serde(default)]
    pub bytes: Option<u64>,
    #[serde(default)]
    pub objects: Option<u64>,
    #[serde(default)]
    pub table_count: Option<u32>,
    pub scanned_at: DateTime<Utc>,
}

fn default_viewer() -> String {
    "viewer".to_string()
}

/// A catalog user, upserted on every sighting via the IAP identity header. The `email` is the
/// merge key (lowercased); `role` is the stored default and is overlaid from the deployment's
/// admin list at read time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserRecord {
    /// UUIDv7, assigned once on first sighting and stable thereafter.
    pub id: String,
    /// Lowercased email; the merge/upsert key.
    pub email: String,
    /// Stored role default. `#[serde(default)]` so a row written before this field existed still
    /// deserializes to "viewer" during a rolling upgrade.
    #[serde(default = "default_viewer")]
    pub role: String,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
}

/// Durable record of a TTL hard-delete.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TtlAuditRecord {
    pub table_id: String,
    pub version_id: String,
    pub deleted_at: DateTime<Utc>,
    #[serde(default)]
    pub reclaimed_bytes: u64,
    #[serde(default)]
    pub policy_snapshot: TtlPolicy,
    /// e.g. "ttl-engine".
    #[serde(default)]
    pub actor: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_version_deserializes_from_an_older_schema_missing_additive_fields() {
        // A row written by an older schema that carried only the identity/structural fields plus
        // the total size -- it lacks the per-component byte splits, the stats options, and the
        // aux list this binary now knows. Every additive field must default rather than fail the
        // whole deserialize (which would otherwise error the registry read during a rolling
        // upgrade). Also carries an unknown future field, which serde must ignore.
        let older_json = r#"{
            "version_id": "2026-01-01T00:00:00Z",
            "timestamp": "2026-01-01T00:00:00Z",
            "snapshot_path": "s3://bucket/t/2026-01-01",
            "shape": "full",
            "storage_bytes_total": 100,
            "synced_at": "2026-01-01T00:00:00Z",
            "some_future_field": {"nested": true}
        }"#;

        let v: TableVersion = serde_json::from_str(older_json).expect("must parse with defaults");
        assert_eq!(v.version_id, "2026-01-01T00:00:00Z");
        assert_eq!(v.shape, VersionShape::Full);
        assert_eq!(v.storage_bytes_total, 100);
        // Additive fields absent in the older row default rather than failing the read.
        assert_eq!(v.lance_core_bytes, 0);
        assert!(!v.partial);
        assert!(!v.protected);
        assert_eq!(v.num_indices, None);
        assert_eq!(v.schema_json, None);
        assert!(v.aux.is_empty());
    }
}
