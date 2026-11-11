import { describe, expect, test } from "vitest";
import {
  buildTimelineRows,
  eventSummary,
  groupByDay,
  operationCounts,
  operationFamily,
  operationLabel,
} from "./timeline";
import type { TimelineEvent } from "./schemas";

function ev(version: number, operation: string, rows: number, ms: number | null = null): TimelineEvent {
  return {
    version,
    timestamp_millis: ms,
    operation,
    summary: { total_rows: String(rows), total_files_size: String(rows * 10), total_fragments: "1", total_deletion_file_rows: "0" },
    writer: null,
    transaction: null,
    tags: [],
    details: {},
  };
}

describe("labels", () => {
  test("maps operations to labels and families", () => {
    expect(operationLabel("Rewrite")).toBe("Compaction");
    expect(operationLabel("CreateIndex")).toBe("Index built");
    expect(operationLabel("Project")).toBe("Schema changed");
    expect(operationLabel("Merge")).toBe("Columns added");
    expect(operationLabel("Mystery")).toBe("Mystery");
    expect(operationFamily("Delete")).toBe("remove");
    expect(operationFamily("Rewrite")).toBe("maintenance");
    expect(operationFamily("Mystery")).toBe("other");
  });
});

describe("buildTimelineRows", () => {
  const events = [ev(10, "Delete", 80), ev(9, "Append", 100), ev(5, "Append", 50)];

  test("computes deltas against the next older loaded event", () => {
    const rows = buildTimelineRows(events);
    expect(rows[0]).toMatchObject({ kind: "event", delta: { rows: -20, bytes: -200, fragments: 0, deletedRows: 0 } });
    expect(rows[1]).toMatchObject({ kind: "event", delta: { rows: 50 } });
    // Oldest loaded entry has no delta.
    expect(rows[3]).toMatchObject({ kind: "event", delta: null });
  });

  test("marks skipped versions as a cleanup gap", () => {
    const rows = buildTimelineRows(events);
    expect(rows.map((r) => r.kind)).toEqual(["event", "event", "gap", "event"]);
    expect(rows[2]).toEqual({ kind: "gap", from: 6, to: 8 });
  });

  test("filters keep full-list deltas and drop gaps", () => {
    const rows = buildTimelineRows(events, (e) => e.operation === "Append");
    expect(rows).toHaveLength(2);
    expect(rows[0]).toMatchObject({ kind: "event", delta: { rows: 50 } });
  });

  test("missing summary values give null deltas", () => {
    const a = ev(2, "Append", 1);
    a.summary = {};
    expect(buildTimelineRows([a, ev(1, "Append", 1)])[0]).toMatchObject({ delta: { rows: null } });
  });
});

test("groupByDay groups by UTC day and keeps gaps with the newer event", () => {
  const day1 = Date.UTC(2026, 10, 10, 12);
  const day0 = Date.UTC(2026, 10, 9, 12);
  const rows = buildTimelineRows([ev(5, "Append", 2, day1), ev(3, "Append", 1, day0), ev(2, "Append", 0, null)]);
  const groups = groupByDay(rows, "utc");
  expect(groups.map((g) => [g.day, g.rows.length])).toEqual([
    ["2026-11-10", 2],
    ["2026-11-09", 1],
    ["unknown", 1],
  ]);
});

test("groupByDay uses the local zone's date", () => {
  // 02:00 UTC on Nov 10 is still Nov 9 at UTC-08:00.
  const rows = buildTimelineRows([ev(1, "Append", 1, Date.UTC(2026, 10, 10, 2))]);
  expect(groupByDay(rows, "local", "Etc/GMT+8")[0].day).toBe("2026-11-09");
});

describe("eventSummary", () => {
  const withDetails = (operation: string, details: Record<string, unknown>) => ({ ...ev(2, operation, 0), details });

  test("prefers the row delta for data changes", () => {
    const [row] = buildTimelineRows([ev(2, "Append", 1500), ev(1, "Append", 500)]);
    expect(row.kind === "event" && eventSummary(row.event, row.delta)).toBe("+1,000 rows");
    expect(eventSummary(ev(2, "Delete", 0), { rows: -20, bytes: 0, fragments: 0, deletedRows: 20 })).toBe("−20 rows");
  });

  test("falls back to recorded details without a delta", () => {
    expect(eventSummary(withDetails("Append", { rows_added: 7 }), null)).toBe("+7 rows");
    expect(eventSummary(withDetails("Delete", { predicate: "id > 3" }), null)).toBe("id > 3");
  });

  test("summarizes metadata and maintenance operations", () => {
    expect(eventSummary(withDetails("CreateIndex", { added: [{ name: "a_idx" }, { name: "b_idx" }] }), null)).toBe(
      "a_idx +1 more",
    );
    expect(
      eventSummary(withDetails("UpdateConfig", { table_metadata: { set: { k: "v" }, removed: [] } }), null),
    ).toBe("k");
    expect(eventSummary(withDetails("Rewrite", { fragments_before: 30, fragments_after: 12 }), null)).toBe(
      "30 → 12 fragments",
    );
    expect(eventSummary(withDetails("Restore", { version: 4 }), null)).toBe("to v4");
    expect(eventSummary(withDetails("Project", {}), null)).toBeNull();
  });
});

test("operationCounts sorts by count", () => {
  expect(operationCounts([ev(3, "Append", 0), ev(2, "Delete", 0), ev(1, "Append", 0)])).toEqual([
    ["Append", 2],
    ["Delete", 1],
  ]);
});
