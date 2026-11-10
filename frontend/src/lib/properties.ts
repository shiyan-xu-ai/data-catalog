export interface PropertyRow {
  key: string;
  value: string;
}

export type BuildPropertiesResult =
  | { ok: true; properties: Record<string, string> }
  | { ok: false; error: string };

/** Merge the optional owner and extra key/value rows into spec `properties`. Rows with an empty
 *  key and value are ignored; duplicate keys (including `owner` when an owner is given) are
 *  rejected rather than silently overwritten. */
export function buildProperties(owner: string, rows: readonly PropertyRow[]): BuildPropertiesResult {
  const properties: Record<string, string> = {};
  const trimmedOwner = owner.trim();
  if (trimmedOwner) properties.owner = trimmedOwner;
  for (const row of rows) {
    const key = row.key.trim();
    if (!key && !row.value.trim()) continue;
    if (!key) return { ok: false, error: "Property keys must not be empty." };
    if (Object.hasOwn(properties, key)) {
      return { ok: false, error: `Duplicate property "${key}".` };
    }
    properties[key] = row.value;
  }
  return { ok: true, properties };
}
