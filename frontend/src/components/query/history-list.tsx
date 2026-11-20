import { useDateTimeFormatter } from "@/lib/preferences";
import type { HistoryEntry } from "@/lib/query-history";

/** The first non-empty line of a query, for a one-line row. */
const firstLine = (sql: string) => sql.split("\n").find((line) => line.trim())?.trim() ?? "";

/** Past runs, newest first. Selecting one hands its text to `onSelect`; it does not run it. */
export function HistoryList({ entries, onSelect }: { entries: readonly HistoryEntry[]; onSelect: (sql: string) => void }) {
  const formatDateTime = useDateTimeFormatter();
  if (entries.length === 0) {
    return <p className="p-4 text-xs text-muted-foreground">No queries run yet</p>;
  }
  return (
    <ul className="min-h-0 flex-1 overflow-y-auto py-1">
      {entries.map((entry) => (
        <li key={entry.sql}>
          <button
            type="button"
            title={entry.sql}
            onClick={() => onSelect(entry.sql)}
            className="block w-full px-3 py-1.5 text-left hover:bg-accent focus-visible:bg-accent focus-visible:outline-hidden"
          >
            <span className="block truncate font-mono text-xs">{firstLine(entry.sql)}</span>
            <span className="block text-[11px] text-muted-foreground tabular-nums">{formatDateTime(new Date(entry.at))}</span>
          </button>
        </li>
      ))}
    </ul>
  );
}
