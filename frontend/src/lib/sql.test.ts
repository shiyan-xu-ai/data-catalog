import { describe, expect, it } from "vitest";
import { defaultQuery, formatCell, quoteIdent, resultsToCsv, shortTypeName, versionRef } from "./sql";

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
