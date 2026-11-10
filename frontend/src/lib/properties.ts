export interface PropertyRow {
  key: string;
  value: string;
}

export type BuildPropertiesResult =
  | { ok: true; properties: Record<string, string> }
  | { ok: false; error: string };

/** Merge optional well-known fields (e.g. owner, description) and extra key/value rows into
 *  spec `properties`. Blank fields and rows with an empty key and value are ignored; duplicate
 *  keys (including a row repeating a set field) and keys under `reservedPrefix` are rejected
 *  rather than silently overwritten. */
export function buildProperties(
  fields: Readonly<Record<string, string>>,
  rows: readonly PropertyRow[],
  reservedPrefix?: string,
): BuildPropertiesResult {
  const properties: Record<string, string> = {};
  for (const [key, value] of Object.entries(fields)) {
    const trimmed = value.trim();
    if (trimmed) properties[key] = trimmed;
  }
  for (const row of rows) {
    const key = row.key.trim();
    if (!key && !row.value.trim()) continue;
    if (!key) return { ok: false, error: "Property keys must not be empty." };
    if (reservedPrefix && key.startsWith(reservedPrefix)) {
      return { ok: false, error: `"${reservedPrefix}" keys are managed by the catalog.` };
    }
    if (Object.hasOwn(properties, key)) {
      return { ok: false, error: `Duplicate property "${key}".` };
    }
    properties[key] = row.value;
  }
  return { ok: true, properties };
}
