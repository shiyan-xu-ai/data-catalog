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

const MAX_TITLE_CHARS = 2000;

function clip(s: string, max: number): string {
  return s.length > max ? `${s.slice(0, max - 1)}…` : s;
}

/** Display Arrow JSON values, keeping text intact for resizable columns. */
export function formatCell(value: unknown, type: string): FormattedCell {
  if (value === null || value === undefined) return { text: "NULL", kind: "null" };
  if (/binary/i.test(type) && (typeof value === "string" || Array.isArray(value))) {
    const text = typeof value === "string" ? value : JSON.stringify(value);
    return { text, kind: "binary", title: clip(text, MAX_TITLE_CHARS) };
  }
  if (typeof value === "boolean") return { text: String(value), kind: "bool" };
  if (typeof value === "number" || typeof value === "bigint") return { text: String(value), kind: "number" };
  if (typeof value === "string") return { text: value, kind: "text" };
  return { text: JSON.stringify(value), kind: "nested" };
}

/** CSV contains the returned rows, including full binary and nested values. */
export function resultsToCsv(columns: readonly { name: string }[], rows: readonly unknown[][]): string {
  const escape = (value: unknown) => {
    const text = value == null ? "" : typeof value === "object" ? JSON.stringify(value) : String(value);
    return /[",\r\n]/.test(text) ? `"${text.replaceAll('"', '""')}"` : text;
  };
  return [columns.map((c) => escape(c.name)), ...rows.map((row) => columns.map((_, i) => escape(row[i])))]
    .map((row) => row.join(","))
    .join("\r\n") + "\r\n";
}

/** Short Arrow type for a column header: parameters of long types collapse to `(…)`. */
export function shortTypeName(type: string): string {
  if (type.length <= 24) return type;
  const paren = type.search(/[(<]/);
  return paren > 0 ? `${type.slice(0, paren)}(…)` : clip(type, 24);
}

/** How SQL names a specific version of the table, e.g. `"events@v12"`. */
export function versionRef(tableName: string, version: number): string {
  return quoteIdent(`${tableName}@v${version}`);
}
