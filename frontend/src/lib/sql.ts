/** Quote a SQL identifier for DataFusion: always double-quoted (unquoted identifiers are folded
 *  to lowercase), with embedded `"` doubled. */
export function quoteIdent(name: string): string {
  return `"${name.replaceAll('"', '""')}"`;
}

/** The query the SQL panel opens with: a small sample of the table. */
export function defaultQuery(tableName: string): string {
  return `SELECT * FROM ${quoteIdent(tableName)} LIMIT 20`;
}

export type CellKind = "null" | "number" | "bool" | "text" | "nested" | "binary";

export interface FormattedCell {
  text: string;
  kind: CellKind;
  /** Fuller value for the hover title when `text` is abbreviated. */
  title?: string;
}

const MAX_CELL_CHARS = 120;
const MAX_TITLE_CHARS = 2000;

function clip(s: string, max: number): string {
  return s.length > max ? `${s.slice(0, max - 1)}…` : s;
}

/** Compact display for one result value. Values follow Arrow's JSON writer: nested types as
 *  JSON objects/arrays and binary as a hex string, recognized through the column type. */
export function formatCell(value: unknown, type: string): FormattedCell {
  if (value === null || value === undefined) return { text: "NULL", kind: "null" };
  if (/binary/i.test(type) && (typeof value === "string" || Array.isArray(value))) {
    const bytes = typeof value === "string" ? Math.ceil(value.length / 2) : value.length;
    const preview = typeof value === "string" ? value : JSON.stringify(value);
    return { text: `<${bytes.toLocaleString("en-US")} bytes>`, kind: "binary", title: clip(preview, MAX_TITLE_CHARS) };
  }
  if (typeof value === "boolean") return { text: String(value), kind: "bool" };
  if (typeof value === "number" || typeof value === "bigint") return { text: String(value), kind: "number" };
  if (typeof value === "string") {
    return value.length > MAX_CELL_CHARS
      ? { text: clip(value, MAX_CELL_CHARS), kind: "text", title: clip(value, MAX_TITLE_CHARS) }
      : { text: value, kind: "text" };
  }
  const json = JSON.stringify(value);
  return {
    text: clip(json, MAX_CELL_CHARS),
    kind: "nested",
    title: json.length > MAX_CELL_CHARS ? clip(JSON.stringify(value, null, 2), MAX_TITLE_CHARS) : undefined,
  };
}

/** Short Arrow type for a column header: parameters of long types collapse to `(…)`. */
export function shortTypeName(type: string): string {
  if (type.length <= 24) return type;
  const paren = type.search(/[(<]/);
  return paren > 0 ? `${type.slice(0, paren)}(…)` : clip(type, 24);
}
