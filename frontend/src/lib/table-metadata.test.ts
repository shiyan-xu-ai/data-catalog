import { describe, expect, it } from "vitest";
import {
  configEntries,
  tableMetadataEntries,
  extraProperties,
  indexCoverage,
  parseManifestSummary,
  totalIndexBytes,
  versionSeries,
  versionStatus,
} from "./table-metadata";
import type { IndexContent } from "./schemas";

const META = {
  total_rows: "8047929",
  total_files_size: "990123456",
  total_fragments: "4486",
  total_data_files: "4486",
  total_deletion_files: "3",
  total_deletion_file_rows: "120",
  "config.lance.auto_cleanup.interval": "20",
  "config.a": "1",
  "table_metadata.producer": "export",
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

describe("configEntries / tableMetadataEntries", () => {
  it("splits the two manifest maps by prefix and sorts", () => {
    expect(configEntries(META)).toEqual([
      ["a", "1"],
      ["lance.auto_cleanup.interval", "20"],
    ]);
    expect(tableMetadataEntries(META)).toEqual([["producer", "export"]]);
    expect(configEntries(null)).toEqual([]);
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

describe("versionStatus", () => {
  it("places a version relative to the served version", () => {
    expect(versionStatus(10, 10)).toBe("served");
    expect(versionStatus(11, 10)).toBe("newer");
    expect(versionStatus(9, 10)).toBe("older");
  });
  it("marks nothing while the served version is unknown", () => {
    expect(versionStatus(11, null)).toBeNull();
    expect(versionStatus(11, undefined)).toBeNull();
  });
});
