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

/** Axis from 0 with at most 4 equal intervals of a "nice" step (1, 2, 2.5, or 5 × 10^k, at
 *  least 1 since rows are whole), using the smallest such step that covers `max`. */
export function niceAxis(max: number): { axisMax: number; ticks: number[] } {
  const target = Math.max(max, 1);
  let magnitude = 10 ** Math.max(0, Math.floor(Math.log10(target / 4)));
  for (;;) {
    for (const m of [1, 2, 2.5, 5]) {
      const step = m * magnitude;
      if (!Number.isInteger(step)) continue;
      const intervals = Math.max(1, Math.ceil(target / step));
      if (intervals <= 4) return { axisMax: step * intervals, ticks: Array.from({ length: intervals + 1 }, (_, i) => i * step) };
    }
    magnitude *= 10;
  }
}

/** Positions (percent of a nice axis from 0) for a rows-per-fragment range plot, plus the
 *  axis ticks. */
export function fragmentBoxPlot(lengths: FragmentLengths): {
  axisMax: number;
  ticks: number[];
  pct: Record<keyof FragmentLengths, number>;
} {
  const { axisMax, ticks } = niceAxis(lengths.max);
  const at = (v: number) => (v / axisMax) * 100;
  return {
    axisMax,
    ticks,
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

/** The table's `lance.compaction.target_rows_per_fragment` override from describe metadata
 *  (`config.` prefixed), or null when unset or not a positive integer. */
export function compactionTargetOverride(meta: Metadata): number | null {
  const v = intOf(meta, "config.lance.compaction.target_rows_per_fragment");
  return v != null && v > 0 ? v : null;
}
