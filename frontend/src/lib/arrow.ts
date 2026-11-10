import type { JsonArrowDataType } from "./schemas";

/** Compact type string for a spec `JsonArrowDataType`, e.g. `list<int32>`, `struct<a: utf8>`,
 *  `fixed_size_list<float32, 128>`, `decimal128(10, 2)`. */
export function formatArrowType(t: JsonArrowDataType): string {
  const children = t.fields ?? [];
  if (children.length === 0) {
    if (t.length == null) return t.type;
    // Lance encodes decimal precision/scale as `precision * 1000 + scale`.
    if (t.type.startsWith("decimal")) {
      return `${t.type}(${Math.floor(t.length / 1000)}, ${t.length % 1000})`;
    }
    return `${t.type}(${t.length})`;
  }
  if (t.type === "struct") {
    return `struct<${children.map((f) => `${f.name}: ${formatArrowType(f.type)}`).join(", ")}>`;
  }
  const lengthSuffix = t.length != null ? `, ${t.length}` : "";
  return `${t.type}<${children.map((f) => formatArrowType(f.type)).join(", ")}${lengthSuffix}>`;
}

/** Like `formatArrowType`, but structs collapse to their field count (`struct<12 fields>`),
 *  for schema-tree rows whose children are shown by expanding the row. */
export function formatArrowTypeShort(t: JsonArrowDataType): string {
  const children = t.fields ?? [];
  if (children.length === 0) return formatArrowType(t);
  if (t.type === "struct") return `struct<${children.length} field${children.length === 1 ? "" : "s"}>`;
  const lengthSuffix = t.length != null ? `, ${t.length}` : "";
  return `${t.type}<${children.map((f) => formatArrowTypeShort(f.type)).join(", ")}${lengthSuffix}>`;
}
