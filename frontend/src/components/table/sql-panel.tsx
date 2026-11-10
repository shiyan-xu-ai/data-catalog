import { useEffect, useMemo, useRef, useState } from "react";
import { CircleHelp, Loader2, PanelRightClose, Play, RotateCcw, Square, TerminalSquare } from "lucide-react";
import { ApiError, runSql } from "@/lib/api";
import { formatCount, parseTime } from "@/lib/format";
import { useDateTimeFormatter } from "@/lib/preferences";
import { ErrorCode, type SqlResponse, type TableVersion } from "@/lib/schemas";
import { defaultQuery, formatCell, quoteIdent, shortTypeName, versionRef, type CellKind } from "@/lib/sql";
import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { SqlEditor } from "@/components/sql-editor";

const cellClass: Record<CellKind, string> = {
  null: "italic text-muted-foreground/60",
  number: "text-right tabular-nums",
  bool: "text-[hsl(var(--chart-3))]",
  text: "",
  nested: "text-muted-foreground",
  binary: "text-[hsl(var(--chart-4))]",
};

function errorTitle(e: Error): string {
  if (!(e instanceof ApiError)) return "Request failed";
  if (e.code === ErrorCode.Throttling) return "The server is busy running other queries; try again shortly";
  if (e.code === ErrorCode.TableNotFound) return "Table not found";
  if (e.code === ErrorCode.TableVersionNotFound) return "Version not found";
  if (e.code === ErrorCode.InvalidInput) return "Query rejected";
  return "Query failed";
}

function formatElapsed(ms: number): string {
  return ms < 1000 ? `${Math.round(ms)} ms` : `${(ms / 1000).toFixed(ms < 10_000 ? 2 : 1)} s`;
}

function ResultGrid({ result }: { result: SqlResponse }) {
  if (result.columns.length === 0) {
    return <p className="p-4 text-xs text-muted-foreground">The query returned no columns.</p>;
  }
  return (
    <table className="w-max min-w-full border-separate border-spacing-0 font-mono text-xs">
      <thead className="sticky top-0 z-10">
        <tr>
          <th className="border-b bg-muted px-2 py-1.5 text-right font-normal text-muted-foreground">#</th>
          {result.columns.map((c, i) => (
            <th key={i} className="border-b border-l bg-muted px-2 py-1.5 text-left align-bottom" title={`${c.name}: ${c.type}`}>
              <div className="font-sans text-xs font-semibold text-foreground">{c.name}</div>
              <div className="text-[10px] font-normal text-muted-foreground">{shortTypeName(c.type)}</div>
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {result.rows.map((row, r) => (
          <tr key={r} className="hover:bg-muted/40">
            <td className="border-b px-2 py-1 text-right text-muted-foreground/70 tabular-nums">{r + 1}</td>
            {result.columns.map((c, i) => {
              const cell = formatCell(row[i], c.type);
              return (
                <td
                  key={i}
                  title={cell.title}
                  className={cn("max-w-[28rem] truncate border-b border-l px-2 py-1 whitespace-nowrap", cellClass[cell.kind])}
                >
                  {cell.text}
                </td>
              );
            })}
          </tr>
        ))}
      </tbody>
    </table>
  );
}

/** Read-only SQL against one table. A version picker sets which version the plain table name
 *  reads (latest by default); other versions are addressable as `"<name>@v<N>"`. The query
 *  runs on the server; Cancel stops waiting for it (the server's own time limit ends it). */
export function SqlPanel({
  segments,
  columns,
  versions,
  latestVersion,
  onCollapse,
}: {
  segments: readonly string[];
  /** Top-level column names, for editor completion. */
  columns: readonly string[];
  /** All versions, newest first (the page's versions query); undefined while loading. */
  versions: readonly TableVersion[] | undefined;
  latestVersion: number | null | undefined;
  onCollapse: () => void;
}) {
  const tableName = segments[segments.length - 1];
  const initial = defaultQuery(tableName);
  const formatDateTime = useDateTimeFormatter();
  const [query, setQuery] = useState(initial);
  // null reads the latest version (the request omits `version`).
  const [version, setVersion] = useState<number | null>(null);
  const [result, setResult] = useState<SqlResponse | null>(null);
  const [error, setError] = useState<Error | null>(null);
  const [startedAt, setStartedAt] = useState<number | null>(null);
  const [now, setNow] = useState(0);
  const controller = useRef<AbortController | null>(null);
  const running = startedAt != null;
  const schema = useMemo(() => ({ [tableName]: columns }), [tableName, columns]);
  const latest = latestVersion ?? versions?.[0]?.version;
  const exampleRef = versionRef(tableName, Math.max(1, (latest ?? 2) - 1));
  const versionHint = `The version picker sets which version ${quoteIdent(tableName)} reads. Reference any other version in SQL as ${exampleRef}, e.g. to compare versions.`;

  // Tick a live elapsed counter while a query is in flight.
  useEffect(() => {
    if (startedAt == null) return;
    setNow(performance.now());
    const id = setInterval(() => setNow(performance.now()), 100);
    return () => clearInterval(id);
  }, [startedAt]);

  // Leaving the page stops waiting for an in-flight query.
  useEffect(() => () => controller.current?.abort(), []);

  async function run() {
    if (running || !query.trim()) return;
    const c = new AbortController();
    controller.current = c;
    setStartedAt(performance.now());
    setError(null);
    try {
      const r = await runSql(segments, query, version, c.signal);
      if (controller.current === c) setResult(r);
    } catch (e) {
      if (controller.current === c && !c.signal.aborted) {
        setResult(null);
        setError(e as Error);
      }
    } finally {
      if (controller.current === c) {
        controller.current = null;
        setStartedAt(null);
      }
    }
  }

  function cancel() {
    controller.current?.abort();
    controller.current = null;
    setStartedAt(null);
  }

  return (
    <div className="flex h-full min-h-0 flex-col overflow-hidden rounded-lg border bg-card text-card-foreground shadow-xs">
      <div className="flex shrink-0 items-center gap-2 border-b px-3 py-2">
        <TerminalSquare className="h-4 w-4 shrink-0 text-muted-foreground" />
        <h2 className="text-sm font-semibold tracking-tight">SQL</h2>
        <Badge variant="outline">Read-only</Badge>
        <div className="ml-auto flex min-w-0 items-center gap-1">
          <select
            aria-label="Table version to query"
            value={version ?? ""}
            onChange={(e) => setVersion(e.target.value === "" ? null : Number(e.target.value))}
            className="h-7 min-w-0 max-w-56 truncate rounded-md border border-input bg-background px-2 text-xs shadow-xs focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
          >
            <option value="">{latest != null ? `Latest (v${latest})` : "Latest"}</option>
            {versions?.map((v) => (
              <option key={v.version} value={v.version}>
                v{v.version}
                {v.timestamp_millis != null ? ` · ${formatDateTime(parseTime(v.timestamp_millis))}` : ""}
              </option>
            ))}
          </select>
          <span className="shrink-0 text-muted-foreground" title={versionHint} aria-label={versionHint}>
            <CircleHelp className="h-3.5 w-3.5" />
          </span>
          <Button variant="ghost" size="icon" className="h-7 w-7 shrink-0" onClick={onCollapse} aria-label="Collapse SQL panel" title="Collapse">
            <PanelRightClose />
          </Button>
        </div>
      </div>

      <SqlEditor value={query} onChange={setQuery} onRun={run} schema={schema} className="h-40 shrink-0 border-b" />

      <div className="flex shrink-0 items-center gap-2 border-b px-3 py-2">
        {running ? (
          <Button size="sm" variant="outline" onClick={cancel}>
            <Square /> Cancel
          </Button>
        ) : (
          <Button size="sm" onClick={run} disabled={!query.trim()}>
            <Play /> Run
          </Button>
        )}
        {query !== initial && !running && (
          <Button size="sm" variant="ghost" onClick={() => setQuery(initial)} title="Reset to the sample query">
            <RotateCcw /> Reset
          </Button>
        )}
        <span className="ml-auto text-[11px] text-muted-foreground">
          {running ? (
            <span className="flex items-center gap-1.5 tabular-nums">
              <Loader2 className="h-3 w-3 animate-spin" /> Running… {formatElapsed(now - (startedAt ?? now))}
            </span>
          ) : (
            <kbd className="rounded border bg-muted px-1 font-mono text-[10px]">⌘/Ctrl ↵</kbd>
          )}
        </span>
      </div>

      <div className={cn("min-h-0 flex-1 overflow-auto", running && "opacity-60")}>
        {error ? (
          <div className="m-3 rounded-md border border-destructive/40 bg-destructive/10 p-3 text-destructive">
            <p className="text-sm font-medium">{errorTitle(error)}</p>
            <pre className="mt-1.5 font-mono text-xs whitespace-pre-wrap break-words">{error.message}</pre>
          </div>
        ) : result ? (
          <ResultGrid result={result} />
        ) : (
          <div className="space-y-2 p-4 text-xs text-muted-foreground">
            <p>
              Run a query to see results. Only <span className="font-mono">SELECT</span> queries against this table run;
              results are capped at 1,000 rows.
            </p>
            <p>
              The picker above sets the version the table name reads. Reference another version as{" "}
              <code className="rounded bg-muted px-1 font-mono text-foreground">{exampleRef}</code>, e.g. to compare
              versions.
            </p>
          </div>
        )}
      </div>

      {result && !error && (
        <div className="flex shrink-0 flex-wrap items-center gap-x-3 gap-y-1 border-t px-3 py-1.5 text-[11px] text-muted-foreground tabular-nums">
          <span>
            {formatCount(result.row_count)} row{result.row_count === 1 ? "" : "s"}
          </span>
          {result.version != null && <span>v{result.version}</span>}
          <span>{formatElapsed(result.elapsed_ms)}</span>
          {result.truncated && (
            <span className="rounded bg-amber-500/15 px-1.5 py-0.5 text-amber-700 dark:text-amber-400">
              Truncated: showing the first {formatCount(result.rows.length)} rows
            </span>
          )}
        </div>
      )}
    </div>
  );
}
