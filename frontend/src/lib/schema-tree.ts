import { formatArrowType, formatArrowTypeShort } from "./arrow";
import type { JsonArrowField } from "./schemas";

export interface SchemaRow {
  /** Dotted path from the top-level column, unique within the schema. */
  path: string;
  name: string;
  depth: number;
  /** Short type for display (`struct<3 fields>`); `fullType` spells out nested types. */
  type: string;
  fullType: string;
  nullable: boolean;
  metadata: [string, string][];
  hasChildren: boolean;
  /** The Arrow field itself, for the raw view. */
  field: JsonArrowField;
}

/** Depth-first rows for a schema tree, descending only into paths in `expanded`. */
export function flattenSchema(
  fields: readonly JsonArrowField[],
  expanded: ReadonlySet<string>,
  depth = 0,
  prefix = "",
): SchemaRow[] {
  const rows: SchemaRow[] = [];
  for (const f of fields) {
    const path = prefix ? `${prefix}.${f.name}` : f.name;
    const children = f.type.fields ?? [];
    rows.push({
      path,
      name: f.name,
      depth,
      type: formatArrowTypeShort(f.type),
      fullType: formatArrowType(f.type),
      nullable: f.nullable,
      metadata: Object.entries(f.metadata ?? {}).sort(([a], [b]) => a.localeCompare(b)),
      hasChildren: children.length > 0,
      field: f,
    });
    if (children.length > 0 && expanded.has(path)) {
      rows.push(...flattenSchema(children, expanded, depth + 1, path));
    }
  }
  return rows;
}

/** Every expandable path, for "expand all". */
export function expandablePaths(fields: readonly JsonArrowField[], prefix = ""): string[] {
  const out: string[] = [];
  for (const f of fields) {
    const children = f.type.fields ?? [];
    if (children.length === 0) continue;
    const path = prefix ? `${prefix}.${f.name}` : f.name;
    out.push(path, ...expandablePaths(children, path));
  }
  return out;
}

/** Schema-level metadata keyed `<key>:<column>` (Lance's `blob-codec:<column>`, for example),
 *  grouped by the top-level column it names as `[key, value]` without the suffix. Entries that
 *  name no column are left out. The longest matching column name wins. */
export function schemaMetadataByColumn(
  meta: Record<string, string> | null | undefined,
  columns: readonly string[],
): Map<string, [string, string][]> {
  const names = [...columns].sort((a, b) => b.length - a.length);
  const byColumn = new Map<string, [string, string][]>();
  for (const [key, value] of Object.entries(meta ?? {}).sort(([a], [b]) => a.localeCompare(b))) {
    const column = names.find((n) => key.length > n.length + 1 && key.endsWith(`:${n}`));
    if (!column) continue;
    const entries = byColumn.get(column) ?? [];
    entries.push([key.slice(0, -(column.length + 1)), value]);
    byColumn.set(column, entries);
  }
  return byColumn;
}
