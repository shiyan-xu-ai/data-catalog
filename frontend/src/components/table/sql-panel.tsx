import { useEffect, useMemo, useRef, useState } from "react";
import { Download, Loader2, PanelLeftClose, PanelLeftOpen, Play, RotateCcw, Square, TerminalSquare } from "lucide-react";
import { ApiError, runSql } from "@/lib/api";
import type { PagedVersions } from "@/lib/catalog-queries";
import { formatCount, parseTime } from "@/lib/format";
import { useDateTimeFormatter } from "@/lib/preferences";
import { ErrorCode, type SqlResponse } from "@/lib/schemas";
import { defaultQuery, formatCell, quoteIdent, resultsToCsv, shortTypeName, versionRef, type CellKind } from "@/lib/sql";
import { versionStatus } from "@/lib/table-metadata";
import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { HelpTooltip } from "@/components/help-tooltip";
import { SqlEditor } from "@/components/sql-editor";
import { SqlDockBar } from "./sql-dock-bar";

// Value of the picker's trailing "load earlier versions" option.
const LOAD_MORE = "more";

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
  const [widths, setWidths] = useState<number[]>([]);
  const widthAt = (index: number) => widths[index] ?? 220;

  function resizeColumn(index: number, width: number) {
    setWidths((current) => {
      const next = [...current];
      next[index] = Math.max(80, Math.min(10000, width));
      return next;
    });
  }

  function startColumnResize(event: React.PointerEvent<HTMLDivElement>, index: number) {
    event.preventDefault();
    const handle = event.currentTarget;
    const startX = event.clientX;
    const startWidth = widthAt(index);
    handle.setPointerCapture(event.pointerId);
    const onMove = (e: PointerEvent) => resizeColumn(index, startWidth + e.clientX - startX);
    const onEnd = () => {
      handle.removeEventListener("pointermove", onMove);
      handle.removeEventListener("pointerup", onEnd);
      handle.removeEventListener("pointercancel", onEnd);
    };
    handle.addEventListener("pointermove", onMove);
    handle.addEventListener("pointerup", onEnd);
    handle.addEventListener("pointercancel", onEnd);
  }

  if (result.columns.length === 0) {
    return <p className="p-4 text-xs text-muted-foreground">The query returned no columns.</p>;
  }
  return (
    <table className="table-fixed border-separate border-spacing-0 font-mono text-xs" style={{ width: 48 + result.columns.reduce((sum, _, i) => sum + widthAt(i), 0) }}>
      <colgroup>
        <col style={{ width: 48 }} />
        {result.columns.map((_, i) => <col key={i} style={{ width: widthAt(i) }} />)}
      </colgroup>
      <thead className="sticky top-0 z-10">
        <tr>
          <th className="border-b bg-muted px-2 py-1.5 text-right font-normal text-muted-foreground">#</th>
          {result.columns.map((c, i) => (
            <th key={i} className="relative overflow-hidden border-b border-l bg-muted px-2 py-1.5 text-left align-bottom" title={`${c.name}: ${c.type}`}>
              <div className="truncate font-sans text-xs font-semibold text-foreground">{c.name}</div>
              <div className="truncate text-[10px] font-normal text-muted-foreground">{shortTypeName(c.type)}</div>
              <div
                role="separator"
                aria-label={`Resize column ${c.name}`}
                aria-orientation="vertical"
                aria-valuenow={widthAt(i)}
                aria-valuemin={80}
                aria-valuemax={10000}
                aria-valuetext={`${widthAt(i)} pixels`}
                tabIndex={0}
                onPointerDown={(e) => startColumnResize(e, i)}
                onKeyDown={(e) => {
                  if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
                    e.preventDefault();
                    resizeColumn(i, widthAt(i) + (e.key === "ArrowLeft" ? -20 : 20));
                  }
                }}
                className="absolute inset-y-0 right-0 w-2 cursor-col-resize touch-none hover:bg-primary/20 focus-visible:bg-primary/20"
              />
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
                  title={cell.title ?? cell.text}
                  className={cn("truncate border-b border-l px-2 py-1 whitespace-nowrap", cellClass[cell.kind])}
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
 *  reads (the served version by default); other versions are addressable as `"<name>@v<N>"`.
 *  The query runs on the server; Cancel stops waiting for it (the server's own time limit ends
 *  it). */
export function SqlPanel({
  segments,
  columns,
  versions,
  servedVersion,
  onCollapse,
}: {
  segments: readonly string[];
  /** Top-level column names, for editor completion. */
  columns: readonly string[];
  /** The page's paged versions query, newest first. */
  versions: PagedVersions;
  servedVersion: number | null | undefined;
  onCollapse: () => void;
}) {
  const tableName = segments[segments.length - 1];
  const initial = defaultQuery(tableName);
  const formatDateTime = useDateTimeFormatter();
  const [query, setQuery] = useState(initial);
  // null reads the served version (the request omits `version`).
  const [version, setVersion] = useState<number | null>(null);
  const [result, setResult] = useState<SqlResponse | null>(null);
  const [error, setError] = useState<Error | null>(null);
  const [startedAt, setStartedAt] = useState<number | null>(null);
  const [now, setNow] = useState(0);
  const [editorCollapsed, setEditorCollapsed] = useState(false);
  const [editorSize, setEditorSize] = useState(40);
  const panes = useRef<HTMLDivElement>(null);
  const controller = useRef<AbortController | null>(null);
  const running = startedAt != null;
  const schema = useMemo(() => ({ [tableName]: columns }), [tableName, columns]);
  const exampleRef = versionRef(tableName, Math.max(1, (servedVersion ?? 2) - 1));
  const versionHint = `The picker sets which version ${quoteIdent(tableName)} reads. Query other versions as ${exampleRef}.`;

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

  function startPaneResize(event: React.PointerEvent<HTMLDivElement>, vertical: boolean) {
    event.preventDefault();
    const handle = event.currentTarget;
    const bounds = panes.current?.getBoundingClientRect();
    if (!bounds) return;
    handle.setPointerCapture(event.pointerId);
    const onMove = (e: PointerEvent) => {
      const percent = vertical
        ? ((e.clientY - bounds.top) / bounds.height) * 100
        : ((e.clientX - bounds.left) / bounds.width) * 100;
      setEditorSize(Math.max(15, Math.min(80, percent)));
    };
    const onEnd = () => {
      handle.removeEventListener("pointermove", onMove);
      handle.removeEventListener("pointerup", onEnd);
      handle.removeEventListener("pointercancel", onEnd);
    };
    handle.addEventListener("pointermove", onMove);
    handle.addEventListener("pointerup", onEnd);
    handle.addEventListener("pointercancel", onEnd);
  }

  function downloadCsv() {
    if (!result) return;
    const url = URL.createObjectURL(new Blob([resultsToCsv(result.columns, result.rows)], { type: "text/csv;charset=utf-8" }));
    const link = document.createElement("a");
    link.href = url;
    link.download = `${tableName.replace(/[\\/:*?"<>|\x00-\x1f]/g, "_") || "results"}.csv`;
    link.click();
    setTimeout(() => URL.revokeObjectURL(url), 0);
  }

  return (
    <div className="flex h-full min-h-0 flex-col text-card-foreground">
      <SqlDockBar
        expanded
        onToggle={onCollapse}
        className="border-b"
        left={
          <>
            <TerminalSquare className="h-4 w-4 shrink-0 text-muted-foreground" />
            <h2 className="text-sm font-semibold tracking-tight">SQL</h2>
            <Badge variant="outline">Read-only</Badge>
            <Button
              size="sm"
              variant="ghost"
              className="h-7"
              aria-expanded={!editorCollapsed}
              aria-label={editorCollapsed ? "Expand SQL editor" : "Collapse SQL editor"}
              onClick={() => setEditorCollapsed(!editorCollapsed)}
            >
              {editorCollapsed ? <PanelLeftOpen /> : <PanelLeftClose />}
              {editorCollapsed ? "Show editor" : "Hide editor"}
            </Button>
            <div className="ml-2 flex items-center gap-2">
              {running ? (
                <Button size="sm" variant="outline" className="h-7" onClick={cancel}>
                  <Square /> Cancel
                </Button>
              ) : (
                <Button size="sm" className="h-7" onClick={run} disabled={!query.trim()}>
                  <Play /> Run
                </Button>
              )}
              {query !== initial && !running && (
                <Button size="sm" variant="ghost" className="h-7" onClick={() => setQuery(initial)} title="Reset to the sample query">
                  <RotateCcw /> Reset
                </Button>
              )}
              <span className="text-[11px] text-muted-foreground">
                {running ? (
                  <span className="flex items-center gap-1.5 tabular-nums">
                    <Loader2 className="h-3 w-3 animate-spin" /> Running… {formatElapsed(now - (startedAt ?? now))}
                  </span>
                ) : (
                  <kbd className="rounded border bg-muted px-1 font-mono text-[10px]">⌘/Ctrl ↵</kbd>
                )}
              </span>
            </div>
          </>
        }
        right={
          <>
            <select
              aria-label="Table version to query"
              value={version ?? ""}
              onChange={(e) => {
                // The trailing option loads another page; the selection stays as it was.
                if (e.target.value === LOAD_MORE) versions.fetchNextPage();
                else setVersion(e.target.value === "" ? null : Number(e.target.value));
              }}
              className="h-7 min-w-0 max-w-64 truncate rounded-md border border-input bg-background px-2 text-xs shadow-xs focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
            >
              <option value="">{servedVersion != null ? `Served (v${servedVersion})` : "Served"}</option>
              {versions.versions.map((v) => (
                <option key={v.version} value={v.version}>
                  v{v.version}
                  {versionStatus(v.version, servedVersion) === "newer" ? " (newer than served)" : ""}
                  {v.timestamp_millis != null ? ` · ${formatDateTime(parseTime(v.timestamp_millis))}` : ""}
                </option>
              ))}
              {versions.hasNextPage && (
                <option value={LOAD_MORE} disabled={versions.isFetchingNextPage}>
                  {versions.isFetchingNextPage ? "Loading earlier versions…" : "Load earlier versions…"}
                </option>
              )}
            </select>
            <HelpTooltip label="About table versions in SQL">{versionHint}</HelpTooltip>
          </>
        }
      />

      <div ref={panes} className="flex min-h-0 flex-1 flex-col md:flex-row">
        <div
          hidden={editorCollapsed}
          className="h-[var(--editor-size)] min-h-0 min-w-0 shrink-0 md:h-auto md:w-[var(--editor-size)]"
          style={{ "--editor-size": `${editorSize}%` } as React.CSSProperties}
        >
          <SqlEditor value={query} onChange={setQuery} onRun={run} schema={schema} className="h-full w-full" />
        </div>
        {!editorCollapsed && ([true, false] as const).map((vertical) => (
          <div
            aria-valuemin={15}
            aria-valuemax={80}
            key={String(vertical)}
            role="separator"
            aria-label="Resize SQL editor"
            aria-orientation={vertical ? "horizontal" : "vertical"}
            aria-valuenow={Math.round(editorSize)}
            tabIndex={0}
            onPointerDown={(e) => startPaneResize(e, vertical)}
            onKeyDown={(e) => {
              const step = vertical ? { ArrowUp: -5, ArrowDown: 5 } : { ArrowLeft: -5, ArrowRight: 5 };
              const delta = step[e.key as keyof typeof step];
              if (delta != null) {
                e.preventDefault();
                setEditorSize((size) => Math.max(15, Math.min(80, size + delta)));
              }
            }}
            className={vertical
              ? "h-2 shrink-0 cursor-row-resize touch-none border-y hover:bg-muted focus-visible:bg-muted md:hidden"
              : "hidden w-2 shrink-0 cursor-col-resize touch-none border-x hover:bg-muted focus-visible:bg-muted md:block"}
          />
        ))}

        <div className="flex min-h-0 min-w-0 flex-1 flex-col">
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
                  Run a query to see results. The server runs only <span className="font-mono">SELECT</span> queries
                  against this table and returns at most 1,000 rows.
                </p>
                <p>
                  The version picker sets the version the table name reads. Reference another version as{" "}
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
              <Button size="sm" variant="ghost" className="ml-auto h-7" onClick={downloadCsv} disabled={running}>
                <Download /> Download CSV
              </Button>
              {result.truncated && (
                <span className="rounded bg-amber-500/15 px-1.5 py-0.5 text-amber-700 dark:text-amber-400">
                  Truncated: showing the first {formatCount(result.rows.length)} rows
                </span>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
