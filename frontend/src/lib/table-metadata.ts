import type { IndexContent, TableStatsResponse, TableVersion } from "./schemas";

type Metadata = Record<string, string> | null | undefined;

// Server-managed catalog properties (`catalog.` prefix is reserved) and ordinary well-known ones.
export const CATALOG_PREFIX = "catalog.";
export const REGISTERED_AT = "catalog.registered_at";
export const UPDATED_AT = "catalog.updated_at";
export const OWNER = "owner";
export const DESCRIPTION = "description";

const CONFIG_PREFIX = "config.";
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

/** Lance table config entries (`config.<key>` in describe metadata), sorted by key. */
export function configEntries(meta: Metadata): [string, string][] {
  return Object.entries(meta ?? {})
    .filter(([k]) => k.startsWith(CONFIG_PREFIX))
    .map(([k, v]): [string, string] => [k.slice(CONFIG_PREFIX.length), v])
    .sort(([a], [b]) => a.localeCompare(b));
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

/** Smallest "nice" axis bound (1, 2, 2.5, or 5 × 10^k) at or above `value`. */
export function niceCeil(value: number): number {
  if (value <= 0) return 1;
  const magnitude = 10 ** Math.floor(Math.log10(value));
  for (const step of [1, 2, 2.5, 5, 10]) {
    if (step * magnitude >= value) return step * magnitude;
  }
  return 10 * magnitude;
}

/** Positions (percent of an axis from 0 to a nice bound above `max`) for a rows-per-fragment
 *  box plot, plus the axis ticks. */
export function fragmentBoxPlot(lengths: FragmentLengths): {
  axisMax: number;
  ticks: number[];
  pct: Record<keyof FragmentLengths, number>;
} {
  const axisMax = niceCeil(lengths.max);
  const at = (v: number) => (v / axisMax) * 100;
  return {
    axisMax,
    ticks: [0, 0.25, 0.5, 0.75, 1].map((f) => f * axisMax),
    pct: {
      min: at(lengths.min),
      p25: at(lengths.p25),
      p50: at(lengths.p50),
      p75: at(lengths.p75),
      p99: at(lengths.p99),
      max: at(lengths.max),
      mean: at(lengths.mean),
    },
  };
}
