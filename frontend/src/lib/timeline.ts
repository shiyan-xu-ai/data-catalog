import { formatCount, formatIsoDate, type TimeZonePref } from "./format";
import type { TimelineEvent } from "./schemas";

/** Operation families share a color on the timeline. */
export type OperationFamily = "write" | "remove" | "update" | "maintenance" | "metadata" | "restore" | "other";

const FAMILY: Record<string, OperationFamily> = {
  Append: "write",
  Overwrite: "write",
  Merge: "write",
  DataReplacement: "write",
  DataOverlay: "write",
  Delete: "remove",
  Update: "update",
  Rewrite: "maintenance",
  CreateIndex: "maintenance",
  ReserveFragments: "maintenance",
  UpdateConfig: "metadata",
  Project: "metadata",
  UpdateMemWalState: "metadata",
  UpdateBases: "metadata",
  Restore: "restore",
  Clone: "restore",
};

const LABEL: Record<string, string> = {
  Append: "Rows appended",
  Overwrite: "Table overwritten",
  Merge: "Columns added",
  DataReplacement: "Data replaced",
  DataOverlay: "Data overlaid",
  Delete: "Rows deleted",
  Update: "Rows updated",
  Rewrite: "Compaction",
  CreateIndex: "Index built",
  ReserveFragments: "Fragments reserved",
  UpdateConfig: "Config updated",
  Project: "Schema changed",
  UpdateMemWalState: "MemWAL updated",
  UpdateBases: "Bases updated",
  Restore: "Restored",
  Clone: "Cloned",
};

export function operationFamily(op: string): OperationFamily {
  return FAMILY[op] ?? "other";
}

export function operationLabel(op: string): string {
  return LABEL[op] ?? (op === "Unknown" ? "Unknown operation" : op);
}

export interface SummaryDelta {
  rows: number | null;
  bytes: number | null;
  fragments: number | null;
  deletedRows: number | null;
}

function num(summary: Record<string, string>, key: string): number | null {
  const v = summary[key];
  if (v == null || v === "") return null;
  const n = Number(v);
  return Number.isFinite(n) ? n : null;
}

function diff(cur: Record<string, string>, prev: Record<string, string>, key: string): number | null {
  const a = num(cur, key);
  const b = num(prev, key);
  return a == null || b == null ? null : a - b;
}

/** Change of `cur` against the version before it. */
export function summaryDelta(cur: TimelineEvent, prev: TimelineEvent): SummaryDelta {
  return {
    rows: diff(cur.summary, prev.summary, "total_rows"),
    bytes: diff(cur.summary, prev.summary, "total_files_size"),
    fragments: diff(cur.summary, prev.summary, "total_fragments"),
    deletedRows: diff(cur.summary, prev.summary, "total_deletion_file_rows"),
  };
}

export type TimelineRow =
  | { kind: "event"; event: TimelineEvent; delta: SummaryDelta | null }
  // Versions `from..=to` were removed by cleanup.
  | { kind: "gap"; from: number; to: number };

/** Rows for loaded events (newest first). Deltas and gaps compare each event with the next
 *  loaded (older) one; the oldest loaded event has no delta until another page loads. With a
 *  filter, deltas still come from the full list and gaps are hidden. */
export function buildTimelineRows(events: readonly TimelineEvent[], keep?: (e: TimelineEvent) => boolean): TimelineRow[] {
  const rows: TimelineRow[] = [];
  for (let i = 0; i < events.length; i++) {
    const event = events[i];
    const prev = events[i + 1];
    if (!keep || keep(event)) rows.push({ kind: "event", event, delta: prev ? summaryDelta(event, prev) : null });
    if (!keep && prev && event.version - prev.version > 1) {
      rows.push({ kind: "gap", from: prev.version + 1, to: event.version - 1 });
    }
  }
  return rows;
}

export interface DayGroup {
  /** `YYYY-MM-DD` in the preferred zone, or `unknown` without a commit time. */
  day: string;
  rows: TimelineRow[];
}

/** Groups rows by commit day in the preferred zone; gaps stay with the newer event above them.
 *  `localZone` pins the zone for `local` (tests). */
export function groupByDay(rows: readonly TimelineRow[], zone: TimeZonePref, localZone?: string): DayGroup[] {
  const groups: DayGroup[] = [];
  for (const row of rows) {
    let current = groups[groups.length - 1];
    if (row.kind === "event" || !current) {
      const ms = row.kind === "event" ? row.event.timestamp_millis : null;
      const day = ms == null ? "unknown" : formatIsoDate(new Date(ms), zone, localZone);
      if (!current || current.day !== day) {
        current = { day, rows: [] };
        groups.push(current);
      }
    }
    current.rows.push(row);
  }
  return groups;
}

export function asString(v: unknown): string | null {
  return typeof v === "string" ? v : typeof v === "number" ? String(v) : null;
}

export function asStrings(v: unknown): string[] {
  return Array.isArray(v) ? v.map(asString).filter((s): s is string => s != null) : [];
}

export function asRecord(v: unknown): Record<string, unknown> | null {
  return v && typeof v === "object" && !Array.isArray(v) ? (v as Record<string, unknown>) : null;
}

function signed(n: number, noun: string): string {
  return `${n > 0 ? "+" : "−"}${formatCount(Math.abs(n))} ${noun}`;
}

function withMore(first: string | undefined, total: number): string | null {
  if (first == null) return null;
  return total > 1 ? `${first} +${total - 1} more` : first;
}

/** Keys an UpdateConfig changed, across config, table metadata, and schema metadata. */
function changedKeys(d: Record<string, unknown>): string[] {
  const keys: string[] = [];
  for (const part of ["table_metadata", "config", "schema_metadata"]) {
    const change = asRecord(d[part]);
    if (!change) continue;
    const set = asRecord(change.set);
    keys.push(...(set ? Object.keys(set) : asStrings(change.set)), ...asStrings(change.removed));
  }
  keys.push(...asStrings(asRecord(d.field_metadata)?.fields));
  return keys;
}

/** One short phrase for a compact timeline row; null when the event has nothing essential. */
export function eventSummary(event: TimelineEvent, delta: SummaryDelta | null): string | null {
  const d = event.details;
  const rows = delta?.rows;
  switch (event.operation) {
    case "Append":
    case "Delete":
    case "Update":
    case "DataReplacement":
      if (rows) return signed(rows, "rows");
      if (event.operation === "Append" && typeof d.rows_added === "number") return signed(d.rows_added, "rows");
      if (event.operation === "Delete") return asString(d.predicate);
      if (event.operation === "Update") return withMore(asStrings(d.fields_modified)[0], asStrings(d.fields_modified).length);
      return null;
    case "Overwrite":
      return typeof d.rows === "number" ? `${formatCount(d.rows)} rows` : null;
    case "CreateIndex": {
      const names = (Array.isArray(d.added) ? d.added : []).map((ix) => asString(asRecord(ix)?.name));
      return withMore(names.find((n) => n != null) ?? undefined, names.length);
    }
    case "UpdateConfig": {
      const keys = changedKeys(d);
      return withMore(keys[0], keys.length);
    }
    case "Rewrite":
      return typeof d.fragments_before === "number" && typeof d.fragments_after === "number"
        ? `${formatCount(d.fragments_before)} → ${formatCount(d.fragments_after)} fragments`
        : null;
    case "Merge":
    case "Project": {
      const fields = asStrings(d.fields);
      if (fields.length) return withMore(fields[0], fields.length);
      return typeof d.fields === "number" ? `${formatCount(d.fields)} columns` : null;
    }
    case "Restore":
      return typeof d.version === "number" ? `to v${d.version}` : null;
    default:
      return rows ? signed(rows, "rows") : null;
  }
}

/** Count of loaded events per operation, most frequent first. */
export function operationCounts(events: readonly TimelineEvent[]): [string, number][] {
  const counts = new Map<string, number>();
  for (const e of events) counts.set(e.operation, (counts.get(e.operation) ?? 0) + 1);
  return [...counts].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
}
