import { describe, expect, it } from "vitest";
import { defaultQuery, formatCell, quoteIdent, resultsToCsv, shortTypeName, versionRef, videoSource } from "./sql";

describe("SQL pre-fill", () => {
  it("always double-quotes the table name and escapes embedded quotes", () => {
    expect(quoteIdent("events")).toBe('"events"');
    expect(quoteIdent("2026-11-07_06-46-14")).toBe('"2026-11-07_06-46-14"');
    expect(quoteIdent('we"ird')).toBe('"we""ird"');
    expect(defaultQuery("2026-11-07_06-46-14")).toBe('SELECT * FROM "2026-11-07_06-46-14" LIMIT 20');
    expect(defaultQuery("MixedCase")).toBe('SELECT * FROM "MixedCase" LIMIT 20');
  });
  it("names a specific version as one quoted identifier", () => {
    expect(versionRef("2026-11-07_06-46-14", 12)).toBe('"2026-11-07_06-46-14@v12"');
    expect(versionRef('we"ird', 3)).toBe('"we""ird@v3"');
  });
});

describe("formatCell", () => {
  it("formats scalars by kind", () => {
    expect(formatCell(null, "Utf8")).toEqual({ text: "NULL", kind: "null" });
    expect(formatCell(42, "Int64")).toEqual({ text: "42", kind: "number" });
    expect(formatCell(false, "Boolean")).toEqual({ text: "false", kind: "bool" });
    expect(formatCell("abc", "Utf8")).toEqual({ text: "abc", kind: "text" });
  });
  it("retains complete text and nested values for resizable columns", () => {
    const long = "x".repeat(500);
    expect(formatCell(long, "Utf8").text).toBe(long);
    expect(formatCell({ data: long }, "Struct").text).toBe(JSON.stringify({ data: long }));
  });
  it("renders nested values as compact JSON", () => {
    expect(formatCell({ a: 1, b: [1, 2] }, "Struct(…)")).toEqual({ text: '{"a":1,"b":[1,2]}', kind: "nested" });
    expect(formatCell([1, 2, 3], "List(Int32)").text).toBe("[1,2,3]");
  });
  it("shows the complete binary value when a column is widened", () => {
    const cell = formatCell("deadbeef", "Binary");
    expect(cell).toMatchObject({ text: "deadbeef", kind: "binary" });
    expect(formatCell("00".repeat(2048), "LargeBinary").text).toBe("00".repeat(2048));
  });
});

describe("shortTypeName", () => {
  it("keeps short types and collapses long parameter lists", () => {
    expect(shortTypeName("Utf8")).toBe("Utf8");
    expect(shortTypeName("Timestamp(µs, None)")).toBe("Timestamp(µs, None)");
    expect(shortTypeName('List(Field { name: "item", data_type: Utf8 })')).toBe("List(…)");
  });
});

describe("resultsToCsv", () => {
  it("exports full values with CSV escaping, JSON nested values, and empty nulls", () => {
    expect(resultsToCsv(
      [{ name: "id" }, { name: 'a,"b"' }, { name: "nested" }],
      [[1, 'hello, "world"\nnext', { x: [1, 2] }], [2, null, "x".repeat(150)]],
    )).toBe('id,"a,""b""",nested\r\n1,"hello, ""world""\nnext","{""x"":[1,2]}"\r\n2,,' + "x".repeat(150) + "\r\n");
  });
});

describe("videoSource", () => {
  it("accepts http(s) and s3 video URLs, with a query string or surrounding space", () => {
    expect(videoSource("https://cdn.example.com/clips/a.mp4")).toEqual({
      url: "https://cdn.example.com/clips/a.mp4",
      rawHevc: false,
      presign: false,
    });
    expect(videoSource("http://host/x/front.H265?X-Amz-Signature=abc&t=1")).toMatchObject({ rawHevc: true, presign: false });
    expect(videoSource(" s3://bucket/media/pack_00000.h265 ")).toEqual({
      url: "s3://bucket/media/pack_00000.h265",
      rawHevc: true,
      presign: true,
    });
  });
  it("rejects other schemes, extensions, and non-strings", () => {
    expect(videoSource("gs://bucket/a.mp4")).toBeNull();
    expect(videoSource("https://host/a.mp4.json")).toBeNull();
    expect(videoSource("https://host/mp4")).toBeNull();
    expect(videoSource({ url: "https://host/a.mp4" })).toBeNull();
  });
});

describe("videoSource for Lance blob columns", () => {
  const blobType =
    'Struct("kind": non-null UInt8, "position": non-null UInt64, "size": non-null UInt64, "blob_id": non-null UInt32, "blob_uri": non-null Utf8)';
  const external = (over: Record<string, unknown> = {}) => ({
    kind: 3,
    blob_id: 0,
    blob_uri: "s3://bucket/media/pack_00000.h265",
    position: 80626,
    size: 1000,
    ...over,
  });
  it("plays an external blob's URI limited to its byte range", () => {
    expect(videoSource(external(), blobType)).toEqual({
      url: "s3://bucket/media/pack_00000.h265",
      rawHevc: true,
      presign: true,
      range: { start: 80626, length: 1000 },
    });
  });
  it("plays the whole object when the blob has no size", () => {
    expect(videoSource(external({ size: 0 }), blobType)).toEqual({
      url: "s3://bucket/media/pack_00000.h265",
      rawHevc: true,
      presign: true,
    });
  });
  it("needs a supported URI suffix and an absolute external blob", () => {
    expect(videoSource(external({ blob_uri: "s3://bucket/media/pack_00000.bin" }), blobType)).toBeNull();
    expect(videoSource(external({ kind: 1 }), blobType)).toBeNull();
    expect(videoSource(external({ blob_id: 2 }), blobType)).toBeNull();
    expect(videoSource(null, blobType)).toBeNull();
  });
  it("reads a descriptor only from a blob column, and a URL string from any column", () => {
    expect(videoSource(external(), 'Struct("a": Int32)')).toBeNull();
    expect(videoSource("s3://bucket/a.mp4", blobType)).toBeNull();
    expect(videoSource("s3://bucket/a.mp4", "Utf8")).toMatchObject({ url: "s3://bucket/a.mp4" });
  });
});
