//! Core registry types: tables, versions, namespaces, aux entries, TTL policy/audit.

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
    pub path: String,
    pub format: AuxFormat,
    /// Convention-derived role (defaults to `name`).
    pub role: String,
    pub storage_bytes: u64,
    /// Cheap fingerprint (e.g. LIST-derived hash) for mixed/unknown formats.
    pub fingerprint: Option<String>,
}

/// Shape of a swept table version, per findings.md.
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
    pub partial: bool,
    /// True when the version is exempt from TTL.
    pub protected: bool,
    pub storage_bytes_total: u64,
    pub lance_core_bytes: u64,
    pub sidecar_bytes: u64,
    pub segments_bytes: u64,
    pub other_aux_bytes: u64,
    pub row_count: Option<u64>,
    pub num_fragments: Option<u64>,
    pub schema_json: Option<String>,
    pub num_indices: Option<u64>,
    pub aux: Vec<AuxEntry>,
    pub swept_at: DateTime<Utc>,
}

/// A table's registry entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableEntry {
    pub id: String,
    pub name: String,
    pub namespace: Namespace,
    pub root_location: String,
    pub owner: Option<String>,
    pub ttl_policy: Option<TtlPolicy>,
    pub last_swept: Option<DateTime<Utc>>,
    pub versions: Vec<TableVersion>,
    /// Latest version's aux summary, kept denormalized for cheap listing.
    pub aux_latest: Vec<AuxEntry>,
}

/// Per-table TTL policy. A version is TTL-eligible only if it exceeds BOTH configured
/// thresholds (kept if within either); no fields set means no TTL applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TtlPolicy {
    pub keep_last_n: Option<u32>,
    pub max_age_days: Option<u32>,
}

/// Durable record of a TTL hard-delete.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TtlAuditRecord {
    pub table_id: String,
    pub version_id: String,
    pub deleted_at: DateTime<Utc>,
    pub reclaimed_bytes: u64,
    pub policy_snapshot: TtlPolicy,
    /// e.g. "ttl-engine".
    pub actor: String,
}
