import { describe, expect, it } from "vitest";
import {
  auxEntrySchema,
  auxSampleResponseSchema,
  errorResponseSchema,
  meResponseSchema,
  storagePrefixStatSchema,
  tableEntrySchema,
  tableVersionSchema,
  ttlAuditRecordSchema,
  ttlPolicySchema,
} from "./schemas";

describe("ttlPolicySchema", () => {
  it("defaults null fields to null", () => {
    const p = ttlPolicySchema.parse({ keep_last_n: 3 });
    expect(p).toEqual({ keep_last_n: 3, max_age_days: null });
  });
  it("defaults both missing to null", () => {
    expect(ttlPolicySchema.parse({})).toEqual({ keep_last_n: null, max_age_days: null });
  });
});

describe("tableVersionSchema", () => {
  it("parses a minimal version with required identity fields only", () => {
    const v = tableVersionSchema.parse({
      version_id: "2026-01-01T00:00:00Z",
      timestamp: "2026-01-01T00:00:00Z",
      snapshot_path: "s3://b/t/2026-01-01",
      shape: "full",
      synced_at: "2026-01-01T00:00:00Z",
    });
    expect(v.partial).toBe(false);
    expect(v.protected).toBe(false);
    expect(v.aux).toEqual([]);
    expect(v.storage_bytes_total).toBe(0);
    expect(v.row_count).toBeNull();
  });
  it("rejects unknown shape enum", () => {
    expect(() =>
      tableVersionSchema.parse({
        version_id: "v",
        timestamp: "t",
        snapshot_path: "p",
        shape: "bogus",
        synced_at: "t",
      }),
    ).toThrow();
  });
});

describe("tableEntrySchema", () => {
  it("parses a full entry", () => {
    const e = tableEntrySchema.parse({
      id: "t1",
      name: "t1",
      namespace: ["ns"],
      root_location: "s3://b/t1",
      owner: "team",
      ttl_policy: { keep_last_n: 2 },
      last_synced: "2026-01-01T00:00:00Z",
      versions: [],
      aux_latest: [],
    });
    expect(e.ttl_policy?.max_age_days).toBeNull();
    expect(e.namespace).toEqual(["ns"]);
  });
  it("accepts null owner and ttl_policy", () => {
    const e = tableEntrySchema.parse({
      id: "t2",
      name: "t2",
      namespace: [],
      root_location: "s3://b/t2",
      owner: null,
      ttl_policy: null,
      last_synced: null,
      versions: [],
      aux_latest: [],
    });
    expect(e.owner).toBeNull();
    expect(e.ttl_policy).toBeNull();
    expect(e.last_synced).toBeNull();
  });
});

describe("auxEntrySchema", () => {
  it("defaults additive fields", () => {
    const a = auxEntrySchema.parse({ name: "segments", format: "parquet" });
    expect(a.path).toBe("");
    expect(a.storage_bytes).toBe(0);
    expect(a.fingerprint).toBeNull();
    expect(a.category).toBeNull();
    expect(a.dataset_path).toBeNull();
    expect(a.row_count).toBeNull();
  });

  it("parses enriched nested-sidecar fields", () => {
    const a = auxEntrySchema.parse({
      name: "dataset.lance/tag_datasets/x.lance/inner/segment_tags.lance",
      format: "lance",
      category: "nested_sidecar",
      dataset_path: "s3://b/.../segment_tags.lance",
      row_count: 42,
      writer_version: "lance/8.0.0",
    });
    expect(a.category).toBe("nested_sidecar");
    expect(a.row_count).toBe(42);
    expect(a.writer_version).toBe("lance/8.0.0");
  });
});

describe("auxSampleResponseSchema", () => {
  it("parses rows + schema and defaults empties", () => {
    const s = auxSampleResponseSchema.parse({
      table_id: "t",
      version_id: "v",
      aux_name: "segments",
      format: "parquet",
      schema: [{ name: "id", data_type: "Int32", nullable: false }],
      rows: [{ id: 1 }, { id: 2 }],
    });
    expect(s.rows).toHaveLength(2);
    expect(s.schema[0].name).toBe("id");

    const empty = auxSampleResponseSchema.parse({
      table_id: "t",
      version_id: "v",
      aux_name: "x",
      format: "lance",
    });
    expect(empty.rows).toEqual([]);
    expect(empty.schema).toEqual([]);
  });
});

describe("ttlAuditRecordSchema", () => {
  it("parses with defaults", () => {
    const r = ttlAuditRecordSchema.parse({
      table_id: "t",
      version_id: "v",
      deleted_at: "2026-01-01T00:00:00Z",
      policy_snapshot: {},
    });
    expect(r.reclaimed_bytes).toBe(0);
    expect(r.actor).toBe("");
    expect(r.policy_snapshot.keep_last_n).toBeNull();
  });
});

describe("errorResponseSchema", () => {
  it("parses error code + message", () => {
    const e = errorResponseSchema.parse({ error_code: 5, message: "not found" });
    expect(e.error_code).toBe(5);
    expect(e.message).toBe("not found");
  });
});

describe("storagePrefixStatSchema", () => {
  it("parses registered and unexplored rows", () => {
    const reg = storagePrefixStatSchema.parse({
      region: "r1", bucket: "b1", prefix: "ns1", registered: true,
      bytes: 10, objects: 2, table_count: 1, scanned_at: "2026-07-05T00:00:00Z",
    });
    expect(reg.bytes).toBe(10);
    const unex = storagePrefixStatSchema.parse({
      region: "r1", bucket: "b1", prefix: "junk", registered: false,
      scanned_at: "2026-07-05T00:00:00Z",
    });
    expect(unex.bytes).toBeNull();
  });
});

describe("meResponseSchema", () => {
  it("accepts anonymous and named callers", () => {
    expect(meResponseSchema.parse({ email: null, role: "viewer" }).email).toBeNull();
    expect(meResponseSchema.parse({ email: "a@x.co", role: "admin" }).role).toBe("admin");
  });
});
