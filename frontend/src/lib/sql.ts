import type { SQLNamespace } from "@codemirror/lang-sql";
import type { ByteRange } from "./hevc-player";
import { joinId } from "./ids";
import type { SqlResponse } from "./schemas";

type SqlColumn = SqlResponse["columns"][number];

/** Quote a SQL identifier for DataFusion: always double-quoted (unquoted identifiers are folded
 *  to lowercase), with embedded `"` doubled. */
export function quoteIdent(name: string): string {
  return `"${name.replaceAll('"', '""')}"`;
}

/** A table's full catalog path as SQL names it: every segment double-quoted, joined by dots. */
export function quotePath(segments: readonly string[]): string {
  return segments.map(quoteIdent).join(".");
}

/** The query a new tab for `segments` opens with: a small sample of the table. */
export function sampleQuery(segments: readonly string[]): string {
  return `SELECT * FROM ${quotePath(segments)} LIMIT 20`;
}

// A run of two or more quoted identifiers joined by dots: a candidate table path.
const QUOTED_PATH = /"(?:[^"]|"")*"(?:\s*\.\s*"(?:[^"]|"")*")+/g;
const QUOTED_PART = /"((?:[^"]|"")*)"/g;
const VERSION_PIN = /@v\d+$/;
const KEY_SEP = "\u0000";

/** The known tables whose quoted full path appears in `text`, in order of first appearance. A
 *  leaf pinned to a version (`"events@v12"`) names its table. */
export function findTablePaths(text: string, tables: readonly (readonly string[])[]): string[][] {
  const known = new Map(tables.map((t) => [t.join(KEY_SEP), t]));
  const found = new Map<string, string[]>();
  for (const [path] of text.matchAll(QUOTED_PATH)) {
    const parts = [...path.matchAll(QUOTED_PART)].map((m) => m[1].replaceAll('""', '"'));
    const leaf = parts[parts.length - 1];
    for (const name of [leaf, leaf.replace(VERSION_PIN, "")]) {
      const key = [...parts.slice(0, -1), name].join(KEY_SEP);
      const table = known.get(key);
      if (table) {
        found.set(key, [...table]);
        break;
      }
    }
  }
  return [...found.values()];
}

type SchemaNode = { [name: string]: SchemaNode | string[] };

/** The editor's completion schema: tables nested under their namespaces, each listing its
 *  columns once `columns` (keyed by the table's joined id) has them, and none before. A namespace
 *  that shares its name with a table of the same parent shadows that table's completion. */
export function completionSchema(
  tables: readonly (readonly string[])[],
  columns: ReadonlyMap<string, readonly string[]>,
): SQLNamespace {
  // Null-prototype objects, so a segment named `constructor` or `__proto__` is an ordinary key.
  const make = (): SchemaNode => Object.create(null);
  const root = make();
  for (const table of tables) {
    let node = root;
    for (const segment of table.slice(0, -1)) {
      const child = node[segment];
      node = Array.isArray(child) || child === undefined ? (node[segment] = make()) : child;
    }
    const leaf = table[table.length - 1];
    if (!(leaf in node)) node[leaf] = [...(columns.get(joinId(table)) ?? [])];
  }
  return root;
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

const VIDEO_URL = /^(https?|s3):\/\/\S+\.(mp4|m4v|mov|webm|h265|hevc)(?:[?#]\S*)?$/i;

/** Queries read a blob column as its stored descriptor (`kind`, `position`, `size`, `blob_id`,
 *  `blob_uri`), whose field metadata carries this flag, including through aliases. */
const BLOB_DESCRIPTOR_KEY = "lance-encoding:blob";

/** Lance `BlobKind::External`: `blob_uri` is an absolute URI when `blob_id` is 0, else a path
 *  under a registered base. The other kinds live in the table's own files, which a descriptor
 *  does not locate. */
const BLOB_KIND_EXTERNAL = 3;

/** The descriptor struct a query reads a blob column as. */
const BLOB_DESCRIPTOR_TYPE =
  /^Struct\("kind": (?:non-null )?UInt8, "position": (?:non-null )?UInt64, "size": (?:non-null )?UInt64, "blob_id": (?:non-null )?UInt32, "blob_uri": (?:non-null )?Utf8\)$/;

/** Whether a result column holds blob descriptors: its field metadata says so (this survives
 *  aliases), and its type is the descriptor struct. */
export function isBlobColumn(column: SqlColumn): boolean {
  return column.metadata?.[BLOB_DESCRIPTOR_KEY] === "true" && BLOB_DESCRIPTOR_TYPE.test(column.type);
}

/** A playable video referenced by a result value. */
export interface VideoSource {
  url: string;
  /** Raw H.265 stream (no container): decoded in the browser instead of a `<video>` element. */
  rawHevc: boolean;
  /** `s3://` objects play from a presigned https URL the catalog signs. */
  presign: boolean;
  /** The bytes of the object the clip occupies, from a blob descriptor's position and size;
   *  absent when the clip is the whole object. */
  range?: ByteRange;
}

function urlSource(url: string): VideoSource | null {
  const match = VIDEO_URL.exec(url);
  if (!match) return null;
  return { url, rawHevc: /^(h265|hevc)$/i.test(match[2]), presign: match[1].toLowerCase() === "s3" };
}

/** The clip an external blob descriptor with an absolute URI points to. The URI must pass the
 *  same check as a plain URL string. A non-zero `size` selects `[position, position + size)`;
 *  a zero `size` means the object's length, so playback runs from `position` to its end. */
function blobSource(value: unknown): VideoSource | null {
  if (typeof value !== "object" || value === null) return null;
  const { kind, position, size, blob_id: blobId, blob_uri: uri } = value as Record<string, unknown>;
  if (kind !== BLOB_KIND_EXTERNAL || blobId !== 0 || typeof uri !== "string") return null;
  const source = urlSource(uri.trim());
  if (!source) return null;
  const start = typeof position === "number" ? position : 0;
  const length = typeof size === "number" && size > 0 ? size : undefined;
  return length != null || start > 0 ? { ...source, range: { start, length } } : source;
}

/** The video a result value points to, or null. A value of a blob column (see `isBlobColumn`)
 *  is read through its descriptor: an external blob whose URI ends in a video extension,
 *  limited to the blob's byte range. Any other value must be an http(s) or s3 URL string ending
 *  in a video extension (a query string allowed). */
export function videoSource(value: unknown, blob = false): VideoSource | null {
  if (blob) return blobSource(value);
  return typeof value === "string" ? urlSource(value.trim()) : null;
}
