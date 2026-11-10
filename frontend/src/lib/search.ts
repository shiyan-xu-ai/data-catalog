import { ID_DELIMITER, displayName } from "./ids";
import { createLimiter } from "./limit";

export interface SearchEntry {
  kind: "namespace" | "table";
  segments: string[];
}

/** Walk the namespace tree from the root: every namespace as a segment array, stopping
 *  after `max` namespaces. `list` returns a namespace's child names; at most `concurrency`
 *  list calls run at once. */
export async function enumerateNamespaces(
  list: (parent: string[]) => Promise<readonly string[]>,
  { max, concurrency }: { max: number; concurrency: number },
): Promise<string[][]> {
  const run = createLimiter(concurrency);
  const found: string[][] = [];
  async function visit(parent: string[]): Promise<void> {
    const names = await run(() => list(parent));
    const children: string[][] = [];
    for (const name of names) {
      if (found.length >= max) break;
      const child = [...parent, name];
      found.push(child);
      children.push(child);
    }
    await Promise.all(children.map(visit));
  }
  await visit([]);
  return found;
}

/** Namespace and table entries for global search: the given namespaces plus every ancestor of
 *  a known table, deduplicated. */
export function buildSearchIndex(
  tables: readonly (readonly string[])[],
  namespaces: readonly (readonly string[])[],
): SearchEntry[] {
  const nsSeen = new Map<string, string[]>();
  const addNs = (segs: readonly string[]) => {
    for (let i = 1; i <= segs.length; i++) {
      const prefix = segs.slice(0, i);
      const id = prefix.join(ID_DELIMITER);
      if (!nsSeen.has(id)) nsSeen.set(id, prefix);
    }
  };
  namespaces.forEach(addNs);
  tables.forEach((t) => addNs(t.slice(0, -1)));
  return [
    ...[...nsSeen.values()].map((segments): SearchEntry => ({ kind: "namespace", segments })),
    ...tables.map((t): SearchEntry => ({ kind: "table", segments: [...t] })),
  ];
}

function score(entry: SearchEntry, q: string): number | null {
  const name = entry.segments[entry.segments.length - 1].toLowerCase();
  if (name === q) return 0;
  if (name.startsWith(q)) return 1;
  if (name.includes(q)) return 2;
  if (displayName(entry.segments).toLowerCase().includes(q)) return 3;
  return null;
}

/** Name matches ranked exact, prefix, substring, then dotted-path substring; ties prefer
 *  shallower entries, then alphabetical order. */
export function searchCatalog(entries: readonly SearchEntry[], query: string, limit: number): SearchEntry[] {
  const q = query.trim().toLowerCase();
  if (!q) return [];
  return entries
    .map((e) => ({ e, s: score(e, q) }))
    .filter((x): x is { e: SearchEntry; s: number } => x.s !== null)
    .sort(
      (a, b) =>
        a.s - b.s ||
        a.e.segments.length - b.e.segments.length ||
        displayName(a.e.segments).localeCompare(displayName(b.e.segments)),
    )
    .slice(0, limit)
    .map((x) => x.e);
}
