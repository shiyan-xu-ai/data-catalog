import type { ByteRange } from "./hevc-player";
import type { JsonArrowField, SqlResponse } from "./schemas";

type SqlColumn = SqlResponse["columns"][number];

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

const VIDEO_URL = /^(https?|s3):\/\/\S+\.(mp4|m4v|mov|webm|h265|hevc)(?:[?#]\S*)?$/i;

/** A Lance blob v2 column carries this extension name in its table-schema field metadata. */
const BLOB_EXTENSION = "lance.blob.v2";
/** Queries read a blob column as its stored descriptor (`kind`, `position`, `size`, `blob_id`,
 *  `blob_uri`), whose field metadata carries this flag, including through aliases. */
const BLOB_DESCRIPTOR_KEY = "lance-encoding:blob";

/** Lance `BlobKind::External`: `blob_uri` is an absolute URI when `blob_id` is 0, else a path
 *  under a registered base. The other kinds live in the table's own files, which a descriptor
 *  does not locate. */
const BLOB_KIND_EXTERNAL = 3;

/** Blob v2 logical children, in order (Blob v2 spec): `data`, `uri`, and optionally `position`
 *  and `size`. */
const BLOB_LOGICAL_CHILDREN = [
  ["data", "large_binary"],
  ["uri", "utf8"],
  ["position", "uint64"],
  ["size", "uint64"],
] as const;

/** The descriptor struct a query reads a blob column as. */
const BLOB_DESCRIPTOR_TYPE =
  /^Struct\("kind": (?:non-null )?UInt8, "position": (?:non-null )?UInt64, "size": (?:non-null )?UInt64, "blob_id": (?:non-null )?UInt32, "blob_uri": (?:non-null )?Utf8\)$/;

function isBlobField(field: JsonArrowField): boolean {
  const children = field.type.fields ?? [];
  return (
    field.metadata?.["ARROW:extension:name"] === BLOB_EXTENSION &&
    field.type.type === "struct" &&
    (children.length === 2 || children.length === 4) &&
    children.every((c, i) => c.name === BLOB_LOGICAL_CHILDREN[i][0] && c.type.type === BLOB_LOGICAL_CHILDREN[i][1])
  );
}

/** Names of the table's top-level blob v2 columns: fields tagged `lance.blob.v2` whose struct
 *  children are the blob v2 logical ones. */
export function blobColumnNames(fields: readonly JsonArrowField[]): Set<string> {
  return new Set(fields.filter(isBlobField).map((f) => f.name));
}

/** Whether a result column holds blob descriptors: its field metadata says so, or it is one of
 *  the table's blob columns by name; either way its type must be the descriptor struct. */
export function isBlobColumn(column: SqlColumn, blobColumns: ReadonlySet<string>): boolean {
  const marked = column.metadata?.[BLOB_DESCRIPTOR_KEY] === "true" || blobColumns.has(column.name);
  return marked && BLOB_DESCRIPTOR_TYPE.test(column.type);
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
