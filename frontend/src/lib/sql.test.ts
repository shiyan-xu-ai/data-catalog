import { describe, expect, it } from "vitest";
import { defaultQuery, formatCell, quoteIdent, shortTypeName } from "./sql";

describe("SQL pre-fill", () => {
  it("always double-quotes the table name and escapes embedded quotes", () => {
    expect(quoteIdent("events")).toBe('"events"');
    expect(quoteIdent("2026-11-07_06-46-14")).toBe('"2026-11-07_06-46-14"');
    expect(quoteIdent('we"ird')).toBe('"we""ird"');
    expect(defaultQuery("2026-11-07_06-46-14")).toBe('SELECT * FROM "2026-11-07_06-46-14" LIMIT 20');
    expect(defaultQuery("MixedCase")).toBe('SELECT * FROM "MixedCase" LIMIT 20');
  });
});

describe("formatCell", () => {
  it("formats scalars by kind", () => {
    expect(formatCell(null, "Utf8")).toEqual({ text: "NULL", kind: "null" });
    expect(formatCell(42, "Int64")).toEqual({ text: "42", kind: "number" });
    expect(formatCell(false, "Boolean")).toEqual({ text: "false", kind: "bool" });
    expect(formatCell("abc", "Utf8")).toEqual({ text: "abc", kind: "text" });
  });
  it("abbreviates long strings with the full value in the title", () => {
    const long = "x".repeat(500);
    const cell = formatCell(long, "Utf8");
    expect(cell.text).toHaveLength(120);
    expect(cell.text.endsWith("…")).toBe(true);
    expect(cell.title).toBe(long);
  });
  it("renders nested values as compact JSON", () => {
    expect(formatCell({ a: 1, b: [1, 2] }, "Struct(…)")).toEqual({ text: '{"a":1,"b":[1,2]}', kind: "nested" });
    expect(formatCell([1, 2, 3], "List(Int32)").text).toBe("[1,2,3]");
  });
  it("renders binary as a byte count, never the payload", () => {
    const cell = formatCell("deadbeef", "Binary");
    expect(cell).toMatchObject({ text: "<4 bytes>", kind: "binary" });
    expect(formatCell("00".repeat(2048), "LargeBinary").text).toBe("<2,048 bytes>");
  });
});

describe("shortTypeName", () => {
  it("keeps short types and collapses long parameter lists", () => {
    expect(shortTypeName("Utf8")).toBe("Utf8");
    expect(shortTypeName("Timestamp(µs, None)")).toBe("Timestamp(µs, None)");
    expect(shortTypeName('List(Field { name: "item", data_type: Utf8 })')).toBe("List(…)");
  });
});
