import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { SQLNamespace } from "@codemirror/lang-sql";
import { Download, Eraser, History, Loader2, PanelTopClose, PanelTopOpen, Play, Square } from "lucide-react";
import { cancelSql, runSql } from "@/lib/api";
import { formatCount } from "@/lib/format";
import { dragPointer } from "@/lib/drag-pointer";
import { displayName } from "@/lib/ids";
import type { HistoryEntry } from "@/lib/query-history";
import type { SqlResponse } from "@/lib/schemas";
import { resultsToCsv } from "@/lib/sql";
import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { SqlEditor, type SqlEditorHandle } from "@/components/sql-editor";
import { HistoryList } from "./history-list";
import { ResultGrid, errorTitle, formatElapsed } from "./result-grid";

export type RunStatus = "idle" | "running" | "failed";

// Workspace width from which the editor sits beside the results, and results width from which
// a selected row's pane sits beside the grid. Widths, not the viewport, because the table
// navigation takes room from the workspace.
const SIDE_BY_SIDE_MIN = 768;
const INLINE_ROW_PANE_MIN = 900;

/** The element's width, kept while it is hidden (a hidden element measures 0). */
function useElementWidth(ref: React.RefObject<HTMLElement | null>): number {
  const [width, setWidth] = useState(0);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    setWidth(el.getBoundingClientRect().width);
    const observer = new ResizeObserver(([entry]) => {
      if (entry.contentRect.width > 0) setWidth(entry.contentRect.width);
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, [ref]);
  return width;
}

function TablesRead({ tables }: { tables: SqlResponse["tables"] }) {
  if (tables.length === 0) return null;
  const list = tables.map((t) => `${displayName(t.id)}@v${t.version}`).join("\n");
  return <span title={list}>{tables.length === 1 ? `v${tables[0].version}` : `${tables.length} tables`}</span>;
}

function truncationLabel(by: SqlResponse["truncated_by"]): string {
  if (by === "rows") return "Truncated by rows (row limit)";
  if (by === "bytes") return "Truncated by bytes (size limit)";
  return "Truncated";
}

/** Best effort: the caller has already stopped waiting, and a 404 (unconfirmed) or failure shows
 *  nothing; the server's time limit still ends a query this misses. */
function stopOnServer(queryId: string) {
  cancelSql(queryId).catch(() => {});
}

/** One query tab's workspace: editor, results, and their toolbar. The query runs on the server;
 *  Cancel, unmounting (closing the tab), and leaving the page stop waiting and ask the server to
 *  stop it. */
export function QueryPane({
  text,
  onTextChange,
  schema,
  history,
  onRun,
  id,
  onStatus,
  editorHandle,
}: {
  id: number;
  text: string;
  onTextChange: (text: string) => void;
  schema: SQLNamespace;
  history: readonly HistoryEntry[];
  onRun: (sql: string) => void;
  onStatus: (id: number, status: RunStatus) => void;
  editorHandle?: React.Ref<SqlEditorHandle>;
}) {
  const [result, setResult] = useState<SqlResponse | null>(null);
  const [runs, setRuns] = useState(0);
  const [error, setError] = useState<Error | null>(null);
  const [startedAt, setStartedAt] = useState<number | null>(null);
  const [now, setNow] = useState(0);
  const [editorCollapsed, setEditorCollapsed] = useState(false);
  const [editorSize, setEditorSize] = useState(40);
  const [historyOpen, setHistoryOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const panes = useRef<HTMLDivElement>(null);
  const results = useRef<HTMLDivElement>(null);
  const controller = useRef<AbortController | null>(null);
  const queryId = useRef<string | null>(null);
  const running = startedAt != null;
  const sideBySide = useElementWidth(root) >= SIDE_BY_SIDE_MIN;
  const inlineRowPane = useElementWidth(results) >= INLINE_ROW_PANE_MIN;
  const status: RunStatus = running ? "running" : error ? "failed" : "idle";

  useEffect(() => {
    onStatus(id, status);
  }, [id, status, onStatus]);

  useEffect(() => {
    if (startedAt == null) return;
    setNow(performance.now());
    const timer = setInterval(() => setNow(performance.now()), 100);
    return () => clearInterval(timer);
  }, [startedAt]);

  useEffect(() => {
    const onPageHide = () => {
      if (queryId.current) stopOnServer(queryId.current);
    };
    window.addEventListener("pagehide", onPageHide);
    return () => {
      window.removeEventListener("pagehide", onPageHide);
      controller.current?.abort();
      if (queryId.current) stopOnServer(queryId.current);
    };
  }, []);

  async function run() {
    if (running || !text.trim()) return;
    const c = new AbortController();
    const qid = crypto.randomUUID();
    controller.current = c;
    queryId.current = qid;
    onRun(text);
    setStartedAt(performance.now());
    setError(null);
    try {
      const r = await runSql(text, qid, c.signal);
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
        queryId.current = null;
        setStartedAt(null);
      }
    }
  }

  function cancel() {
    controller.current?.abort();
    controller.current = null;
    if (queryId.current) stopOnServer(queryId.current);
    queryId.current = null;
    setStartedAt(null);
  }

  function startPaneResize(event: React.PointerEvent<HTMLDivElement>) {
    const bounds = panes.current?.getBoundingClientRect();
    if (!bounds) {
      event.preventDefault();
      return;
    }
    dragPointer(event, (e) => {
      // Stacked, the editor's height is in viewport units; side by side, its width is a share of the panes'.
      const percent = sideBySide
        ? ((e.clientX - bounds.left) / bounds.width) * 100
        : ((e.clientY - bounds.top) / window.innerHeight) * 100;
      setEditorSize(Math.max(15, Math.min(80, percent)));
    });
  }

  function downloadCsv() {
    if (!result) return;
    const url = URL.createObjectURL(new Blob([resultsToCsv(result.columns, result.rows)], { type: "text/csv;charset=utf-8" }));
    const link = document.createElement("a");
    link.href = url;
    link.download = "query-results.csv";
    link.click();
    setTimeout(() => URL.revokeObjectURL(url), 0);
  }

  return (
    <div ref={root} className="flex min-h-0 min-w-0 flex-1 flex-col text-card-foreground">
      <div className="flex min-h-10 shrink-0 flex-wrap items-center gap-2 border-b bg-card px-4 py-1.5">
        <Badge variant="outline">Read-only</Badge>
        <Button
          size="sm"
          variant="ghost"
          className="h-7"
          aria-expanded={!editorCollapsed}
          aria-label={editorCollapsed ? "Expand SQL editor" : "Collapse SQL editor"}
          onClick={() => setEditorCollapsed(!editorCollapsed)}
        >
          {editorCollapsed ? <PanelTopOpen /> : <PanelTopClose />}
          {editorCollapsed ? "Show editor" : "Hide editor"}
        </Button>
        <div className="ml-2 flex items-center gap-2">
          {running ? (
            <Button size="sm" variant="outline" className="h-7" onClick={cancel}>
              <Square /> Cancel
            </Button>
          ) : (
            <Button size="sm" className="h-7" onClick={run} disabled={!text.trim()}>
              <Play /> Run
            </Button>
          )}
          {text.length > 0 && !running && (
            <Button size="sm" variant="ghost" className="h-7" onClick={() => onTextChange("")} title="Clear the query">
              <Eraser /> Clear
            </Button>
          )}
          <Popover open={historyOpen} onOpenChange={setHistoryOpen}>
            <PopoverTrigger asChild>
              <Button size="sm" variant="ghost" className="h-7">
                <History /> History
              </Button>
            </PopoverTrigger>
            <PopoverContent className="flex max-h-[min(24rem,var(--radix-popover-content-available-height))] w-[min(22rem,calc(100vw-2rem))] flex-col">
              <HistoryList
                entries={history}
                onSelect={(sql) => {
                  onTextChange(sql);
                  setHistoryOpen(false);
                }}
              />
            </PopoverContent>
          </Popover>
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
      </div>

      <div ref={panes} className={cn("flex min-h-0 flex-1", sideBySide ? "flex-row" : "flex-col")}>
        <div
          hidden={editorCollapsed}
          className="min-h-0 min-w-0 shrink-0 overflow-hidden"
          style={sideBySide ? { width: `${editorSize}%` } : { height: `${editorSize}vh` }}
        >
          <SqlEditor
            value={text}
            onChange={onTextChange}
            onRun={run}
            schema={schema}
            handle={editorHandle}
            className="h-full w-full"
          />
        </div>
        {!editorCollapsed && (
          <div
            aria-valuemin={15}
            aria-valuemax={80}
            role="separator"
            aria-label="Resize SQL editor"
            aria-orientation={sideBySide ? "vertical" : "horizontal"}
            aria-valuenow={Math.round(editorSize)}
            tabIndex={0}
            onPointerDown={startPaneResize}
            onKeyDown={(e) => {
              const step = sideBySide ? { ArrowLeft: -5, ArrowRight: 5 } : { ArrowUp: -5, ArrowDown: 5 };
              const delta = step[e.key as keyof typeof step];
              if (delta != null) {
                e.preventDefault();
                setEditorSize((size) => Math.max(15, Math.min(80, size + delta)));
              }
            }}
            className={cn(
              "shrink-0 touch-none hover:bg-muted focus-visible:bg-muted",
              sideBySide ? "w-2 cursor-col-resize border-x" : "h-2 cursor-row-resize border-y",
            )}
          />
        )}

        <div ref={results} className={cn("flex min-h-0 min-w-0 flex-1 flex-col", running && "opacity-60")}>
          {result && !error && (
            <div className="flex shrink-0 flex-wrap items-center gap-x-3 gap-y-1 border-b px-3 py-1.5 text-[11px] text-muted-foreground tabular-nums">
              <span>
                {formatCount(result.row_count)} row{result.row_count === 1 ? "" : "s"}
              </span>
              <TablesRead tables={result.tables} />
              <span>{formatElapsed(result.elapsed_ms)}</span>
              {result.cached && (
                <span className="rounded bg-muted px-1.5 py-0.5" title="Served from the server's result cache; the tables read have not changed">
                  cached
                </span>
              )}
              {result.truncated && (
                <span className="rounded bg-amber-500/15 px-1.5 py-0.5 text-amber-700 dark:text-amber-400">
                  {truncationLabel(result.truncated_by)}: showing the first {formatCount(result.rows.length)} rows
                </span>
              )}
              <Button size="sm" variant="ghost" className="ml-auto h-7" onClick={downloadCsv} disabled={running}>
                <Download /> Download CSV
              </Button>
            </div>
          )}
          <div data-pane-scroll className="min-h-0 flex-1 overflow-y-auto">
            {error ? (
              <div className="m-3 rounded-md border border-destructive/40 bg-destructive/10 p-3 text-destructive">
                <p className="text-sm font-medium">{errorTitle(error)}</p>
                <pre className="mt-1.5 font-mono text-xs whitespace-pre-wrap break-words">{error.message}</pre>
              </div>
            ) : result ? (
              // Keyed per run: a new result starts with no row selected and default column widths.
              <ResultGrid key={runs} result={result} inlinePane={inlineRowPane} />
            ) : (
              <div className="space-y-2 p-4 text-xs text-muted-foreground">
                <p>
                  Run a query to see results. The server runs only <span className="font-mono">SELECT</span> queries
                  and returns at most 1,000 rows.
                </p>
                <p>
                  Name a table by its full quoted path, for example{" "}
                  <code className="rounded bg-muted px-1 font-mono text-foreground">"namespace"."table"</code>. Add{" "}
                  <code className="rounded bg-muted px-1 font-mono text-foreground">@v12</code> to the table name to read
                  that version, as in{" "}
                  <code className="rounded bg-muted px-1 font-mono text-foreground">"namespace"."table@v12"</code>.
                </p>
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
