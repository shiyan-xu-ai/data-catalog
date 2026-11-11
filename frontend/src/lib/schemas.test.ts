import { describe, expect, it } from "vitest";
import {
  columnStorageResponseSchema,
  describeNamespaceResponseSchema,
  describeTableResponseSchema,
  errorResponseSchema,
  listNamespacesResponseSchema,
  listTableBranchesResponseSchema,
  listTableIndicesResponseSchema,
  listTableTagsResponseSchema,
  listTableVersionsResponseSchema,
  listTablesResponseSchema,
} from "./schemas";

describe("table metadata responses", () => {
  it("parses a version page with manifest metadata", () => {
    const r = listTableVersionsResponseSchema.parse({
      versions: [
        { version: 39, manifest_path: "_versions/39.manifest", timestamp_millis: 1, metadata: { total_rows: "5" } },
        { version: 38, manifest_path: "_versions/38.manifest" },
      ],
      page_token: "38",
    });
    expect(r.versions[0].metadata?.total_rows).toBe("5");
    expect(r.page_token).toBe("38");
  });
  it("parses indices with optional sizes and counts", () => {
    const r = listTableIndicesResponseSchema.parse({
      indexes: [{ index_name: "id_idx", index_uuid: "u", columns: ["id"], status: "ok", index_type: "BTREE" }],
    });
    expect(r.indexes[0].size_bytes).toBeUndefined();
  });
  it("parses camelCase tag and branch contents", () => {
    expect(
      listTableTagsResponseSchema.parse({ tags: { v1: { version: 3, manifestSize: 10 } } }).tags.v1.version,
    ).toBe(3);
    expect(
      listTableBranchesResponseSchema.parse({
        branches: { dev: { parentVersion: 2, createAt: 5, manifestSize: 1 } },
      }).branches.dev.parentVersion,
    ).toBe(2);
    expect(listTableTagsResponseSchema.parse({ tags: {} }).tags).toEqual({});
  });
  it("parses the catalog column-storage response", () => {
    const r = columnStorageResponseSchema.parse({ version: 4, columns: [{ name: "a", bytes_on_disk: 12 }] });
    expect(r.columns[0].bytes_on_disk).toBe(12);
  });
  it("keeps detailed describe metadata", () => {
    expect(describeTableResponseSchema.parse({ metadata: { total_rows: "1" } }).metadata).toEqual({ total_rows: "1" });
  });
});

describe("list responses", () => {
  it("parses names with and without a page token", () => {
    expect(listNamespacesResponseSchema.parse({ namespaces: ["a", "b"] }).namespaces).toEqual(["a", "b"]);
    expect(listTablesResponseSchema.parse({ tables: ["t"], page_token: "n" }).page_token).toBe("n");
    expect(listTablesResponseSchema.parse({ tables: [], page_token: null }).tables).toEqual([]);
  });
  it("rejects non-string names", () => {
    expect(() => listNamespacesResponseSchema.parse({ namespaces: [{ name: "a" }] })).toThrow();
  });
});

describe("describeNamespaceResponseSchema", () => {
  it("accepts an empty body and properties", () => {
    expect(describeNamespaceResponseSchema.parse({}).properties).toBeUndefined();
    expect(describeNamespaceResponseSchema.parse({ properties: { owner: "x" } }).properties).toEqual({ owner: "x" });
  });
});

describe("describeTableResponseSchema", () => {
  it("parses a detailed describe with nested schema and stats", () => {
    const t = describeTableResponseSchema.parse({
      table: "t",
      namespace: ["a", "b"],
      location: "s3://bucket/t.lance",
      table_uri: "s3://bucket/t.lance",
      properties: { owner: "team" },
      version: 7,
      schema: {
        fields: [
          { name: "id", type: { type: "int64" }, nullable: false },
          {
            name: "vec",
            type: {
              type: "fixed_size_list",
              fields: [{ name: "item", type: { type: "float32" }, nullable: true }],
              length: 128,
            },
            nullable: true,
            metadata: { k: "v" },
          },
        ],
      },
      stats: { num_deleted_rows: 0, num_fragments: 3 },
    });
    expect(t.version).toBe(7);
    expect(t.schema?.fields[1].type.fields?.[0].type.type).toBe("float32");
    expect(t.stats?.num_fragments).toBe(3);
  });
  it("accepts a cheap describe without detailed metadata", () => {
    const t = describeTableResponseSchema.parse({ location: "s3://b/t", version: null, schema: null });
    expect(t.schema).toBeNull();
    expect(t.stats).toBeUndefined();
  });
  it("rejects a malformed schema field", () => {
    expect(() =>
      describeTableResponseSchema.parse({ schema: { fields: [{ name: "x", type: "int32", nullable: true }] } }),
    ).toThrow();
  });
});

describe("errorResponseSchema", () => {
  it("parses spec error bodies", () => {
    expect(errorResponseSchema.parse({ error: "not empty", code: 3 })).toEqual({ error: "not empty", code: 3 });
    expect(errorResponseSchema.parse({ code: 18 }).code).toBe(18);
  });
});
