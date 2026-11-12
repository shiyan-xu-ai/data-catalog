import { useEffect, useMemo, useRef, useState } from "react";
import { Download, Loader2, PanelLeftClose, PanelLeftOpen, Play, RotateCcw, Square } from "lucide-react";
import { ApiError, runSql } from "@/lib/api";
import type { PagedVersions } from "@/lib/catalog-queries";
import { formatCount, parseTime } from "@/lib/format";
import { useDateTimeFormatter } from "@/lib/preferences";
import { ErrorCode, type SqlResponse } from "@/lib/schemas";
import { defaultQuery, formatCell, isVideoUrl, quoteIdent, resultsToCsv, shortTypeName, versionRef, type CellKind } from "@/lib/sql";
import { versionStatus } from "@/lib/table-metadata";
import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { CopyButton } from "@/components/copy-button";
import { HelpTooltip } from "@/components/help-tooltip";
import { JsonTree } from "@/components/json-tree";
import { PaneTabs, SidePane, SidePaneLayout, usePaneSelection } from "@/components/side-pane";
import { SqlEditor } from "@/components/sql-editor";

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

// Side by side, the editor and its divider stay in view below the toolbar while the results
// scroll with the page; they fill the rest of the viewport below the 3.5rem top bar, the tab bar,
// and the toolbar, above the page's 1.5rem bottom padding.
const PINNED =
  "md:sticky md:top-[calc(var(--tabs-h,0px)+var(--sticky-top,0px))] md:h-[calc(100dvh-5rem-var(--tabs-h,0px)-var(--sticky-top,0px))] md:self-start";

const rowIndex = (i: number) => i;

function copyText(value: unknown): string {
  if (value == null) return "";
  return typeof value === "object" ? JSON.stringify(value, null, 2) : String(value);
}

type RowView = "values" | "playback";

/** One playable column: a native video player, or a note and link when the browser cannot
 *  decode the file (raw H.265 streams, for example). */
function VideoClip({ name, url }: { name: string; url: string }) {
  const [failed, setFailed] = useState(false);
  return (
    <section className="space-y-1.5 border-t px-4 py-3">
      <div className="flex min-w-0 items-center gap-2">
        <span className="truncate font-mono text-xs font-medium" title={name}>
          {name}
        </span>
        <a href={url} target="_blank" rel="noreferrer" className="ml-auto shrink-0 text-xs text-muted-foreground underline-offset-2 hover:text-foreground hover:underline">
          Open
        </a>
      </div>
      {failed ? (
        <p className="text-xs text-muted-foreground">This browser cannot play this file. Open it to download.</p>
      ) : (
        <video controls preload="metadata" src={url} onError={() => setFailed(true)} className="aspect-video w-full rounded bg-black" />
      )}
    </section>
  );
}

/** Every column of one result row with its type and full value, nested values as a JSON tree.
 *  When some value is a video URL (see `isVideoUrl`), a Playback view shows a player per such
 *  column. */
function RowPane({
  result,
  row,
  top,
  view,
  onView,
  onClose,
}: {
  result: SqlResponse;
  row: number;
  top: number;
  view: RowView;
  onView: (view: RowView) => void;
  onClose: () => void;
}) {
  const values = result.rows[row];
  const videos = result.columns.flatMap((c, i) => {
    const value = values[i];
    return isVideoUrl(value) ? [{ name: c.name, url: value.trim() }] : [];
  });
  const shown = videos.length > 0 ? view : "values";
  return (
    <SidePane
      label={`Row ${row + 1} details`}
      top={top}
      onClose={onClose}
      bodyKey={`${row}:${shown}`}
      wide={shown === "playback"}
      className="xl:mt-3 xl:mr-3"
      header={
        <>
          <div className="font-medium">Row {formatCount(row + 1)}</div>
          <div className="text-xs text-muted-foreground">of {formatCount(result.rows.length)} returned</div>
        </>
      }
      toolbar={
        videos.length > 0 && (
          <PaneTabs
            views={[
              ["values", "Values"],
              ["playback", `Playback (${videos.length})`],
            ]}
            value={shown}
            onChange={onView}
          />
        )
      }
    >
      {shown === "playback"
        ? videos.map((v, i) => <VideoClip key={i} name={v.name} url={v.url} />)
        : result.columns.map((c, i) => {
            const cell = formatCell(values[i], c.type);
            return (
              <section key={i} className="space-y-1 border-t px-4 py-2.5">
                <div className="flex min-w-0 items-center gap-2">
                  <span className="truncate font-mono text-xs font-medium" title={c.name}>
                    {c.name}
                  </span>
                  <span className="truncate text-[10px] text-muted-foreground" title={c.type}>
                    {shortTypeName(c.type)}
                  </span>
                  <span className="ml-auto">
                    <CopyButton value={() => copyText(values[i])} label={`Copy ${c.name}`} />
                  </span>
                </div>
                {cell.kind === "nested" ? (
                  <div className="overflow-x-auto">
                    <JsonTree name={c.name} value={values[i]} />
                  </div>
                ) : (
                  <div className={cn("font-mono text-xs break-all whitespace-pre-wrap", cellClass[cell.kind], "text-left")}>
                    {cell.text}
                  </div>
                )}
              </section>
            );
          })}
    </SidePane>
  );
}

/** Results with a header that pins below the tab bar and the toolbar (`--sticky-top`) while the
 *  page scrolls. Wide results scroll sideways in the body; the header, a separate table with the
 *  same column widths, follows that scroll. Selecting a row opens its values beside the grid,
 *  pinned `paneTop` px below the tab bar. */
function ResultGrid({ result, paneTop }: { result: SqlResponse; paneTop: number }) {
  const [widths, setWidths] = useState<number[]>([]);
  const head = useRef<HTMLDivElement>(null);
  const indices = useMemo(() => result.rows.map((_, i) => i), [result]);
  const { current, close, rowProps } = usePaneSelection(indices, rowIndex);
  const [view, setView] = useState<RowView>("values");
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
  const width = 48 + result.columns.reduce((sum, _, i) => sum + widthAt(i), 0);
  const columns = (
    <colgroup>
      <col style={{ width: 48 }} />
      {result.columns.map((_, i) => <col key={i} style={{ width: widthAt(i) }} />)}
    </colgroup>
  );
  const tableClass = "table-fixed border-separate border-spacing-0 font-mono text-xs";
  return (
    <SidePaneLayout
      pane={
        current != null && (
          <RowPane result={result} row={current} top={paneTop} view={view} onView={setView} onClose={close} />
        )
      }
    >
      <div ref={head} className="sticky top-[calc(var(--tabs-h,0px)+var(--sticky-top,0px)-1px)] z-10 overflow-hidden">
        <table className={tableClass} style={{ width, minWidth: "100%" }}>
          {columns}
          <thead>
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
        </table>
      </div>
      <div
        className="overflow-x-auto"
        onScroll={(e) => {
          if (head.current) head.current.scrollLeft = e.currentTarget.scrollLeft;
        }}
      >
        <table className={tableClass} style={{ width, minWidth: "100%" }}>
          {columns}
          <tbody>
            {result.rows.map((row, r) => (
              <tr
                key={r}
                {...rowProps(r)}
                aria-selected={r === current}
                className={cn(
                  "cursor-pointer outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset",
                  r === current ? "bg-accent" : "hover:bg-muted/40",
                )}
              >
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
      </div>
    </SidePaneLayout>
  );
}

/** Read-only SQL against one table. A version picker sets which version the plain table name
 *  reads (the current version by default); other versions are addressable as `"<name>@v<N>"`.
 *  The query runs on the server; Cancel stops waiting for it (the server's own time limit ends
 *  it). */
export function SqlPanel({
  segments,
  columns,
  versions,
  currentVersion,
}: {
  segments: readonly string[];
  /** Top-level column names, for editor completion. */
  columns: readonly string[];
  /** The page's paged versions query, newest first. */
  versions: PagedVersions;
  currentVersion: number | null | undefined;
}) {
  const tableName = segments[segments.length - 1];
  const initial = defaultQuery(tableName);
  const formatDateTime = useDateTimeFormatter();
  const [query, setQuery] = useState(initial);
  // null reads the current version (the request omits `version`).
  const [version, setVersion] = useState<number | null>(null);
  const [result, setResult] = useState<SqlResponse | null>(null);
  const [runs, setRuns] = useState(0);
  const [error, setError] = useState<Error | null>(null);
  const [startedAt, setStartedAt] = useState<number | null>(null);
  const [now, setNow] = useState(0);
  const [editorCollapsed, setEditorCollapsed] = useState(false);
  const [editorSize, setEditorSize] = useState(40);
  const panes = useRef<HTMLDivElement>(null);
  const toolbar = useRef<HTMLDivElement>(null);
  const [toolbarHeight, setToolbarHeight] = useState(0);
  const controller = useRef<AbortController | null>(null);
  const running = startedAt != null;
  const schema = useMemo(() => ({ [tableName]: columns }), [tableName, columns]);
  const exampleRef = versionRef(tableName, Math.max(1, (currentVersion ?? 2) - 1));
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

  // The editor and result header pin below the toolbar, which wraps on narrow widths.
  useEffect(() => {
    const el = toolbar.current;
    if (!el) return;
    const observer = new ResizeObserver(() => setToolbarHeight(el.offsetHeight));
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  async function run() {
    if (running || !query.trim()) return;
    const c = new AbortController();
    controller.current = c;
    setStartedAt(performance.now());
    setError(null);
    try {
      const r = await runSql(segments, query, version, c.signal);
      if (controller.current === c) {
        setResult(r);
        setRuns((n) => n + 1);
      }
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
      // Stacked panes size the editor by viewport height; side by side, by the panes' width.
      const percent = vertical
        ? ((e.clientY - bounds.top) / window.innerHeight) * 100
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
    <div className="text-card-foreground" style={{ "--sticky-top": `${toolbarHeight}px` } as React.CSSProperties}>
      <div
        ref={toolbar}
        className="sticky top-(--tabs-h,0px) z-20 flex min-h-10 flex-wrap items-center gap-2 rounded-t-lg border-b bg-card px-4 py-1.5"
      >
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
        <div className="ml-auto flex min-w-0 items-center gap-1.5">
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
            <option value="">{currentVersion != null ? `Current (v${currentVersion})` : "Current"}</option>
            {versions.versions.map((v) => (
              <option key={v.version} value={v.version}>
                v{v.version}
                {versionStatus(v.version, currentVersion) === "newer" ? " (newer than current)" : ""}
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
        </div>
      </div>

      <div ref={panes} className="flex flex-col md:flex-row">
        <div
          hidden={editorCollapsed}
          className={cn("h-[calc(var(--editor-size)*1vh)] min-w-0 shrink-0 md:w-[calc(var(--editor-size)*1%)]", PINNED)}
          style={{ "--editor-size": editorSize } as React.CSSProperties}
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
              : cn("hidden w-2 shrink-0 cursor-col-resize touch-none border-x hover:bg-muted focus-visible:bg-muted md:block", PINNED)}
          />
        ))}

        <div className={cn("min-w-0 flex-1", running && "opacity-60")}>
          {result && !error && (
            <div className="flex flex-wrap items-center gap-x-3 gap-y-1 border-b px-3 py-1.5 text-[11px] text-muted-foreground tabular-nums">
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
              <Button size="sm" variant="ghost" className="ml-auto h-7" onClick={downloadCsv} disabled={running}>
                <Download /> Download CSV
              </Button>
            </div>
          )}
          {error ? (
            <div className="m-3 rounded-md border border-destructive/40 bg-destructive/10 p-3 text-destructive">
              <p className="text-sm font-medium">{errorTitle(error)}</p>
              <pre className="mt-1.5 font-mono text-xs whitespace-pre-wrap break-words">{error.message}</pre>
            </div>
          ) : result ? (
            // Keyed per run: a new result starts with no row selected and default column widths.
            <ResultGrid key={runs} result={result} paneTop={toolbarHeight} />
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
      </div>
    </div>
  );
}
