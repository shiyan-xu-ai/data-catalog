import { describe, expect, it } from "vitest";
import { expandablePaths, flattenSchema, schemaMetadataByColumn } from "./schema-tree";
import type { JsonArrowField } from "./schemas";

const FIELDS: JsonArrowField[] = [
  { name: "id", type: { type: "int64" }, nullable: false, metadata: { b: "2", a: "1" } },
  {
    name: "pose",
    type: {
      type: "struct",
      fields: [
        { name: "x", type: { type: "float64" }, nullable: true },
        {
          name: "tags",
          type: { type: "list", fields: [{ name: "item", type: { type: "utf8" }, nullable: true }] },
          nullable: true,
        },
      ],
    },
    nullable: true,
  },
];

describe("flattenSchema", () => {
  it("shows only top-level rows when nothing is expanded", () => {
    const rows = flattenSchema(FIELDS, new Set());
    expect(rows.map((r) => r.path)).toEqual(["id", "pose"]);
    expect(rows[0].metadata).toEqual([
      ["a", "1"],
      ["b", "2"],
    ]);
    expect(rows[1]).toMatchObject({
      hasChildren: true,
      type: "struct<2 fields>",
      fullType: "struct<x: float64, tags: list<utf8>>",
    });
  });
  it("descends into expanded paths with depth", () => {
    const rows = flattenSchema(FIELDS, new Set(["pose", "pose.tags"]));
    expect(rows.map((r) => [r.path, r.depth])).toEqual([
      ["id", 0],
      ["pose", 0],
      ["pose.x", 1],
      ["pose.tags", 1],
      ["pose.tags.item", 2],
    ]);
  });
});

describe("expandablePaths", () => {
  it("lists every nested path", () => {
    expect(expandablePaths(FIELDS)).toEqual(["pose", "pose.tags"]);
  });
});

describe("schemaMetadataByColumn", () => {
  it("groups `<key>:<column>` entries under the longest matching column", () => {
    const { byColumn, rest } = schemaMetadataByColumn(
      { "blob-codec:log": "zstd", "codec:a:log": "lz4", "owner": "x", "note:missing": "y" },
      ["log", "a:log"],
    );
    expect(Object.fromEntries(byColumn)).toEqual({ log: [["blob-codec", "zstd"]], "a:log": [["codec", "lz4"]] });
    expect(rest).toEqual([
      ["note:missing", "y"],
      ["owner", "x"],
    ]);
  });
});
