import { describe, expect, it } from "vitest";
import {
  configEntries,
  extraProperties,
  fragmentBoxPlot,
  indexCoverage,
  niceCeil,
  parseManifestSummary,
  storageFormatVersion,
  topColumns,
  totalIndexBytes,
  versionSeries,
} from "./table-metadata";
import type { IndexContent } from "./schemas";

describe("fragment box plot", () => {
  it("rounds the axis up to a nice bound", () => {
    expect(niceCeil(2038)).toBe(2500);
    expect(niceCeil(2000)).toBe(2000);
    expect(niceCeil(1_048_576)).toBe(2_000_000);
    expect(niceCeil(7)).toBe(10);
    expect(niceCeil(0)).toBe(1);
  });
  it("places each statistic as a percent of the axis", () => {
    const plot = fragmentBoxPlot({ min: 5, p25: 1771, p50: 1831, p75: 1879, p99: 1966, max: 2038, mean: 1791 });
    expect(plot.axisMax).toBe(2500);
    expect(plot.ticks).toEqual([0, 625, 1250, 1875, 2500]);
    expect(plot.pct.min).toBeCloseTo(0.2);
    expect(plot.pct.p50).toBeCloseTo(73.24);
    expect(plot.pct.max).toBeCloseTo(81.52);
  });
});

const META = {
  total_rows: "8047929",
  total_files_size: "990123456",
  total_fragments: "4486",
  total_data_files: "4486",
  total_deletion_files: "3",
  total_deletion_file_rows: "120",
  "config.lance.auto_cleanup.interval": "20",
  "config.a": "1",
  "lance.data_storage_version": "2.2",
};

describe("parseManifestSummary", () => {
  it("reads the Lance version summary keys", () => {
    expect(parseManifestSummary(META)).toEqual({
      rows: 8047929,
      dataBytes: 990123456,
      fragments: 4486,
      dataFiles: 4486,
      deletionFiles: 3,
      deletedRows: 120,
    });
  });
  it("leaves absent or malformed values null instead of zero", () => {
    const s = parseManifestSummary({ total_rows: "-1", total_files_size: "abc" });
    expect(s.rows).toBeNull();
    expect(s.dataBytes).toBeNull();
    expect(parseManifestSummary(undefined).fragments).toBeNull();
  });
});

describe("configEntries / storageFormatVersion", () => {
  it("strips the config prefix and sorts", () => {
    expect(configEntries(META)).toEqual([
      ["a", "1"],
      ["lance.auto_cleanup.interval", "20"],
    ]);
    expect(configEntries(null)).toEqual([]);
  });
  it("reads the data storage version", () => {
    expect(storageFormatVersion(META)).toBe("2.2");
    expect(storageFormatVersion({})).toBeNull();
  });
});

describe("extraProperties", () => {
  it("omits owner, description, and the two catalog-recorded times only", () => {
    expect(
      extraProperties({
        owner: "t",
        description: "d",
        "catalog.registered_at": "x",
        "catalog.updated_at": "y",
        "catalog.other": "z",
        tier: "gold",
      }),
    ).toEqual([
      ["catalog.other", "z"],
      ["tier", "gold"],
    ]);
  });
});

const ix = (p: Partial<IndexContent>): IndexContent => ({ index_name: "i", columns: ["c"], ...p });

describe("index helpers", () => {
  it("sums only reported index sizes", () => {
    expect(totalIndexBytes([])).toBeNull();
    expect(totalIndexBytes([ix({}), ix({ size_bytes: 10 }), ix({ size_bytes: 5 })])).toBe(15);
  });
  it("computes coverage only from both counts", () => {
    expect(indexCoverage(ix({ num_indexed_rows: 75, num_unindexed_rows: 25 }))).toBe(0.75);
    expect(indexCoverage(ix({ num_indexed_rows: 75 }))).toBeNull();
    expect(indexCoverage(ix({ num_indexed_rows: 0, num_unindexed_rows: 0 }))).toBeNull();
  });
});

describe("versionSeries", () => {
  it("sorts ascending and extracts rows and bytes", () => {
    expect(
      versionSeries([
        { version: 2, timestamp_millis: 2000, metadata: { total_rows: "20", total_files_size: "200" } },
        { version: 1, metadata: { total_rows: "10" } },
      ]),
    ).toEqual([
      { version: 1, timestamp: null, rows: 10, bytes: null },
      { version: 2, timestamp: 2000, rows: 20, bytes: 200 },
    ]);
  });
});

describe("topColumns", () => {
  it("keeps the largest columns and aggregates the rest", () => {
    const cols = [
      { name: "a", bytes_on_disk: 1 },
      { name: "b", bytes_on_disk: 30 },
      { name: "c", bytes_on_disk: 20 },
      { name: "d", bytes_on_disk: 4 },
    ];
    expect(topColumns(cols, 2)).toEqual([
      { name: "b", bytes: 30 },
      { name: "c", bytes: 20 },
      { name: "2 other columns", bytes: 5, other: true },
    ]);
    expect(topColumns(cols, 10)).toHaveLength(4);
  });
});
