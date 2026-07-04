// API response types — mirror the serde-serialized shapes from catalog-core/src/types.rs
// and catalog-api/src/api.rs. Field names are snake_case exactly as serialized.

export type AuxFormat = "parquet" | "lance" | "csv" | "mixed" | "unknown";

export type VersionShape =
  | "full"
  | "lance_only"
  | "seg_only"
  | "lance_only_partial"
  | "empty";

export interface AuxEntry {
  name: string;
  path: string;
  format: AuxFormat;
  role: string;
  storage_bytes: number;
  fingerprint: string | null;
}

export interface TableVersion {
  version_id: string;
  timestamp: string; // RFC3339
  snapshot_path: string;
  shape: VersionShape;
  partial: boolean;
  protected: boolean;
  storage_bytes_total: number;
  lance_core_bytes: number;
  sidecar_bytes: number;
  segments_bytes: number;
  other_aux_bytes: number;
  row_count: number | null;
  num_fragments: number | null;
  schema_json: string | null;
  num_indices: number | null;
  aux: AuxEntry[];
  swept_at: string; // RFC3339
}

export interface TtlPolicy {
  keep_last_n: number | null;
  max_age_days: number | null;
}

export interface TableEntry {
  id: string;
  name: string;
  namespace: string[]; // Namespace serializes as a JSON array of segments
  root_location: string;
  owner: string | null;
  ttl_policy: TtlPolicy | null;
  last_swept: string | null; // RFC3339, null if never swept
  versions: TableVersion[];
  aux_latest: AuxEntry[];
}

export interface TtlDryRunResponse {
  table_id: string;
  candidates: string[];
  reclaimable_bytes: number;
}

export interface TtlApplyResponse {
  table_id: string;
  deleted: string[];
  reclaimed_bytes: number;
}

export interface TtlAuditRecord {
  table_id: string;
  version_id: string;
  deleted_at: string; // RFC3339
  reclaimed_bytes: number;
  policy_snapshot: TtlPolicy;
  actor: string;
}

export interface ErrorResponse {
  error_code: number;
  message: string;
}

export interface NotLeaderResponse {
  message: string;
}
