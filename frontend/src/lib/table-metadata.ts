import type { IndexContent, TableStatsResponse, TableVersion } from "./schemas";

type Metadata = Record<string, string> | null | undefined;

// Server-managed catalog properties (`catalog.` prefix is reserved) and ordinary well-known ones.
export const REGISTERED_AT = "catalog.registered_at";
export const UPDATED_AT = "catalog.updated_at";
export const OWNER = "owner";
export const DESCRIPTION = "description";

const CONFIG_PREFIX = "config.";
const TABLE_METADATA_PREFIX = "table_metadata.";
const STORAGE_VERSION = "lance.data_storage_version";

/** Lance manifest summary for one version. Fields are null when the key is absent or not a
 *  non-negative integer, so the UI never shows a fabricated zero. */
export interface ManifestSummary {
  rows: number | null;
  dataBytes: number | null;
  fragments: number | null;
  dataFiles: number | null;
  deletionFiles: number | null;
  deletedRows: number | null;
}

function intOf(meta: Metadata, key: string): number | null {
  const v = meta?.[key];
  if (v == null || !/^\d+$/.test(v)) return null;
  return Number(v);
}

export function parseManifestSummary(meta: Metadata): ManifestSummary {
  return {
    rows: intOf(meta, "total_rows"),
    dataBytes: intOf(meta, "total_files_size"),
    fragments: intOf(meta, "total_fragments"),
    dataFiles: intOf(meta, "total_data_files"),
    deletionFiles: intOf(meta, "total_deletion_files"),
    deletedRows: intOf(meta, "total_deletion_file_rows"),
  };
}

function prefixedEntries(meta: Metadata, prefix: string): [string, string][] {
  return Object.entries(meta ?? {})
    .filter(([k]) => k.startsWith(prefix))
    .map(([k, v]): [string, string] => [k.slice(prefix.length), v])
    .sort(([a], [b]) => a.localeCompare(b));
}

/** Lance table config entries (`config.<key>` in describe metadata), sorted by key. */
export function configEntries(meta: Metadata): [string, string][] {
  return prefixedEntries(meta, CONFIG_PREFIX);
}

/** Lance table metadata entries (`table_metadata.<key>` in describe metadata), sorted by key. */
export function tableMetadataEntries(meta: Metadata): [string, string][] {
  return prefixedEntries(meta, TABLE_METADATA_PREFIX);
}

export function storageFormatVersion(meta: Metadata): string | null {
  return meta?.[STORAGE_VERSION] || null;
}

/** Properties shown under "Properties": everything except the fields the details sidebar
 *  renders on their own (owner, description, and the two catalog-recorded times). */
export function extraProperties(props: Metadata): [string, string][] {
  return Object.entries(props ?? {})
    .filter(([k]) => k !== OWNER && k !== DESCRIPTION && k !== REGISTERED_AT && k !== UPDATED_AT)
    .sort(([a], [b]) => a.localeCompare(b));
}

/** Sum of index sizes, or null when no index reports a size. */
export function totalIndexBytes(indexes: readonly IndexContent[]): number | null {
  let total: number | null = null;
  for (const ix of indexes) {
    if (ix.size_bytes != null) total = (total ?? 0) + ix.size_bytes;
  }
  return total;
}

/** Indexed share of rows for an index, or null when counts are missing or both zero. */
export function indexCoverage(ix: IndexContent): number | null {
  const indexed = ix.num_indexed_rows;
  const unindexed = ix.num_unindexed_rows;
  if (indexed == null || unindexed == null || indexed + unindexed === 0) return null;
  return indexed / (indexed + unindexed);
}

export interface VersionPoint {
  version: number;
  timestamp: number | null;
  rows: number | null;
  bytes: number | null;
}

/** Versions in ascending order with their manifest summary, for the growth chart. */
export function versionSeries(versions: readonly TableVersion[]): VersionPoint[] {
  return [...versions]
    .sort((a, b) => a.version - b.version)
    .map((v) => {
      const s = parseManifestSummary(v.metadata);
      return { version: v.version, timestamp: v.timestamp_millis ?? null, rows: s.rows, bytes: s.dataBytes };
    });
}

export interface ColumnSlice {
  name: string;
  bytes: number;
  /** True for the aggregated "other columns" slice. */
  other?: boolean;
}

/** The `limit` largest columns by bytes plus one aggregated slice for the rest. */
export function topColumns(
  columns: readonly { name: string; bytes_on_disk: number }[],
  limit: number,
): ColumnSlice[] {
  const sorted = [...columns].sort((a, b) => b.bytes_on_disk - a.bytes_on_disk);
  const top: ColumnSlice[] = sorted.slice(0, limit).map((c) => ({ name: c.name, bytes: c.bytes_on_disk }));
  const rest = sorted.slice(limit);
  if (rest.length > 0) {
    top.push({
      name: `${rest.length} other column${rest.length === 1 ? "" : "s"}`,
      bytes: rest.reduce((s, c) => s + c.bytes_on_disk, 0),
      other: true,
    });
  }
  return top;
}

/** Lance's default compaction target (`CompactionOptions::target_rows_per_fragment`); the
 *  stats route counts fragments with fewer physical rows than this as small. */
export const COMPACTION_TARGET_ROWS = 1_048_576;

export type FragmentLengths = TableStatsResponse["fragment_stats"]["lengths"];

/** The table's `lance.compaction.target_rows_per_fragment` override from describe metadata
 *  (`config.` prefixed), or null when unset or not a positive integer. */
export function compactionTargetOverride(meta: Metadata): number | null {
  const v = intOf(meta, "config.lance.compaction.target_rows_per_fragment");
  return v != null && v > 0 ? v : null;
}

/** Where a version sits relative to the served version (the one reads without a version
 *  answer for), or null while the served version is unknown. */
export function versionStatus(
  version: number,
  served: number | null | undefined,
): "served" | "newer" | "older" | null {
  if (served == null) return null;
  if (version === served) return "served";
  return version > served ? "newer" : "older";
}
