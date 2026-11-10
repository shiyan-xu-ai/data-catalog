import { describe, expect, it } from "vitest";
import { formatArrowType, formatArrowTypeShort } from "./arrow";

describe("formatArrowType", () => {
  it("formats primitives and sized types", () => {
    expect(formatArrowType({ type: "int32" })).toBe("int32");
    expect(formatArrowType({ type: "fixed_size_binary", length: 16 })).toBe("fixed_size_binary(16)");
    expect(formatArrowType({ type: "decimal128", length: 10002 })).toBe("decimal128(10, 2)");
  });
  it("formats nested types", () => {
    const item = { name: "item", type: { type: "float32" }, nullable: true };
    expect(formatArrowType({ type: "list", fields: [item] })).toBe("list<float32>");
    expect(formatArrowType({ type: "fixed_size_list", fields: [item], length: 4 })).toBe("fixed_size_list<float32, 4>");
    expect(
      formatArrowType({
        type: "struct",
        fields: [
          { name: "a", type: { type: "utf8" }, nullable: true },
          { name: "b", type: { type: "list", fields: [item] }, nullable: false },
        ],
      }),
    ).toBe("struct<a: utf8, b: list<float32>>");
  });
});

describe("formatArrowTypeShort", () => {
  it("collapses structs to a field count but keeps list element types", () => {
    const item = { name: "item", type: { type: "utf8" }, nullable: true };
    const struct = {
      type: "struct",
      fields: [
        { name: "a", type: { type: "utf8" }, nullable: true },
        { name: "b", type: { type: "int32" }, nullable: true },
      ],
    };
    expect(formatArrowTypeShort(struct)).toBe("struct<2 fields>");
    expect(formatArrowTypeShort({ type: "list", fields: [item] })).toBe("list<utf8>");
    expect(formatArrowTypeShort({ type: "list", fields: [{ name: "item", type: struct, nullable: true }] })).toBe(
      "list<struct<2 fields>>",
    );
    expect(formatArrowTypeShort({ type: "int64" })).toBe("int64");
  });
});
