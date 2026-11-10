// Spec ids are path segments joined by `$`; the root namespace is `$`. Route params carry the
// joined form and the UI displays dotted names. Build ids only from segment arrays (KTD8).

export const ID_DELIMITER = "$";
export const ROOT_ID = "$";

/** Why `name` cannot be a namespace/table segment, or null if it is valid. */
export function segmentError(name: string): string | null {
  if (name.length === 0) return "Name is required.";
  if (name.includes(ID_DELIMITER)) return `Name must not contain "${ID_DELIMITER}".`;
  if (name.trim() !== name) return "Name must not have leading or trailing whitespace.";
  return null;
}

export function joinId(segments: readonly string[]): string {
  if (segments.length === 0) return ROOT_ID;
  for (const s of segments) {
    const err = segmentError(s);
    if (err) throw new Error(`invalid id segment ${JSON.stringify(s)}: ${err}`);
  }
  return segments.join(ID_DELIMITER);
}

export function splitId(id: string): string[] {
  if (id === "" || id === ROOT_ID) return [];
  return id.split(ID_DELIMITER);
}

/** Joined id, URL-encoded for use as a single `/v1/...{id}/...` path segment. */
export function idPathSegment(segments: readonly string[]): string {
  return encodeURIComponent(joinId(segments));
}

export function displayName(segments: readonly string[]): string {
  return segments.join(".");
}
