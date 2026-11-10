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
