export interface HistoryEntry {
  sql: string;
  /** Run time, epoch milliseconds. */
  at: number;
}

const KEY = "query-history";
export const MAX_HISTORY = 50;

/** `entries` (newest first) with `sql` run at `at` on top. An entry with the same text moves
 *  to the top with the new time instead of repeating; the oldest entries beyond the cap drop. */
export function addHistory(entries: readonly HistoryEntry[], sql: string, at: number): HistoryEntry[] {
  return [{ sql, at }, ...entries.filter((e) => e.sql !== sql)].slice(0, MAX_HISTORY);
}

const isEntry = (e: unknown): e is HistoryEntry =>
  typeof e === "object" && e !== null && typeof (e as HistoryEntry).sql === "string" && typeof (e as HistoryEntry).at === "number";

/** The saved history; missing, unreadable, or malformed data loads as empty. */
export function loadHistory(): HistoryEntry[] {
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(KEY) ?? "[]");
    return Array.isArray(parsed) ? parsed.filter(isEntry).slice(0, MAX_HISTORY) : [];
  } catch {
    return [];
  }
}

export function saveHistory(entries: readonly HistoryEntry[]): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(entries));
  } catch {
    // Storage unavailable; history still works for this session.
  }
}
