import { useCallback, useEffect, useMemo, useReducer, useRef, useState, useSyncExternalStore } from "react";
import { useNavigate } from "@tanstack/react-router";
import { PanelLeftOpen } from "lucide-react";
import { ActiveContext } from "@/lib/active";
import { useAllTableIds, useTableColumns } from "@/lib/catalog-queries";
import { joinId, splitId } from "@/lib/ids";
import { useStoredState } from "@/lib/preferences";
import { addHistory, loadHistory, saveHistory } from "@/lib/query-history";
import { initialTabs, tabsReducer } from "@/lib/query-tabs";
import { completionSchema, findTablePaths, sampleQuery } from "@/lib/sql";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import type { SqlEditorHandle } from "@/components/sql-editor";
import { LeftPane } from "./left-pane";
import { QueryPane, type RunStatus } from "./query-pane";
import { QueryTabStrip } from "./query-tab-strip";

// Typing in the editor loads the columns of the tables it names once it pauses this long.
const COLUMN_LOAD_DELAY_MS = 400;
const DESKTOP_QUERY = "(min-width: 768px)";

function useDebounced<T>(value: T, delayMs: number): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const id = setTimeout(() => setDebounced(value), delayMs);
    return () => clearTimeout(id);
  }, [value, delayMs]);
  return debounced;
}

function useDesktop(): boolean {
  return useSyncExternalStore(
    (notify) => {
      const media = window.matchMedia(DESKTOP_QUERY);
      media.addEventListener("change", notify);
      return () => media.removeEventListener("change", notify);
    },
    () => window.matchMedia(DESKTOP_QUERY).matches,
  );
}

/** The Query page: a table browser and history on the left; query tabs, an editor, and results in
 *  the workspace. Every tab stays mounted (hidden when inactive) so its text, results, and running
 *  query survive switching tabs, and the root keeps the whole page mounted once visited.
 *
 *  `table` is the `table` search parameter, a joined table id: it opens one tab pre-filled with a
 *  sample query of that table (the initial tab when the page mounts with it) and is then cleared
 *  from the URL. */
export function QueryPage({ active, table }: { active: boolean; table: string | undefined }) {
  const navigate = useNavigate();
  const desktop = useDesktop();
  const [state, dispatch] = useReducer(tabsReducer, table, (t) => initialTabs(t ? sampleQuery(splitId(t)) : ""));
  const { tabs, activeId } = state;
  const [statuses, setStatuses] = useState<Record<number, RunStatus>>({});
  const [history, setHistory] = useState(loadHistory);
  const [collapsed, setCollapsed] = useStoredState("query-pane-collapsed", false);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const editors = useRef(new Map<number, SqlEditorHandle>());

  // The table parameter this page already turned into a tab; it resets once the URL drops it.
  const consumed = useRef(table);
  useEffect(() => {
    if (table == null) {
      consumed.current = undefined;
      return;
    }
    if (consumed.current !== table) {
      consumed.current = table;
      dispatch({ type: "open", text: sampleQuery(splitId(table)) });
    }
    navigate({ to: "/query", search: {}, replace: true });
  }, [table, navigate]);

  useEffect(() => saveHistory(history), [history]);

  const tableIds = useAllTableIds(true);
  // Tables whose columns are wanted: expanded or inserted from the browser (`wanted`), and those
  // the active tab's text names.
  const [wanted, setWanted] = useState<readonly string[]>([]);
  const want = useCallback((id: string) => setWanted((w) => (w.includes(id) ? w : [...w, id])), []);
  const activeText = tabs.find((t) => t.id === activeId)?.text ?? "";
  const settledText = useDebounced(activeText, COLUMN_LOAD_DELAY_MS);
  const named = useMemo(() => findTablePaths(settledText, tableIds.data ?? []).map(joinId), [settledText, tableIds.data]);
  const columnIds = useMemo(() => [...new Set([...wanted, ...named])], [wanted, named]);
  const columns = useTableColumns(columnIds);
  const schema = useMemo(() => {
    const names = new Map<string, string[]>();
    for (const [id, c] of columns) if (c.fields) names.set(id, c.fields.map((f) => f.name));
    return completionSchema(tableIds.data ?? [], names);
  }, [columns, tableIds.data]);

  const recordRun = useCallback((sql: string) => {
    const at = Date.now();
    setHistory((h) => addHistory(h, sql, at));
  }, []);
  const setStatus = useCallback(
    (id: number, status: RunStatus) => setStatuses((s) => (s[id] === status ? s : { ...s, [id]: status })),
    [],
  );

  function insert(text: string, tableId?: string) {
    editors.current.get(activeId)?.insert(text);
    if (tableId) want(tableId);
    setDrawerOpen(false);
  }

  function selectHistory(sql: string) {
    dispatch({ type: "setText", id: activeId, text: sql });
    setDrawerOpen(false);
  }

  const leftPane = (docked: boolean) => (
    <LeftPane
      docked={docked}
      collapsed={collapsed}
      onCollapsedChange={setCollapsed}
      onClose={() => setDrawerOpen(false)}
      browser={{ columns, onWantColumns: want, onInsert: insert }}
      history={{ entries: history, onSelect: selectHistory }}
    />
  );

  return (
    <div className="flex h-full min-h-0 bg-card">
      {desktop && leftPane(true)}
      {!desktop && drawerOpen && (
        <div className="fixed inset-0 z-50 flex">
          {leftPane(false)}
          <button type="button" aria-label="Close tables and history" className="flex-1 bg-black/40" onClick={() => setDrawerOpen(false)} />
        </div>
      )}
      <div className="flex min-w-0 flex-1 flex-col">
        <div className="flex shrink-0 items-center border-b bg-muted/30">
          {!desktop && (
            <Button
              variant="ghost"
              size="icon"
              className="mx-1 h-7 w-7 shrink-0"
              aria-label="Open tables and history"
              onClick={() => setDrawerOpen(true)}
            >
              <PanelLeftOpen />
            </Button>
          )}
          <QueryTabStrip
            tabs={tabs}
            activeId={activeId}
            statuses={statuses}
            onActivate={(id) => dispatch({ type: "activate", id })}
            onClose={(id) => dispatch({ type: "close", id })}
            onNew={() => dispatch({ type: "open" })}
          />
        </div>
        {tabs.map((tab) => (
          <div
            key={tab.id}
            role="tabpanel"
            id={`query-panel-${tab.id}`}
            aria-labelledby={`query-tab-${tab.id}`}
            className={cn("min-h-0 flex-1 flex-col", tab.id === activeId ? "flex" : "hidden")}
          >
            <ActiveContext.Provider value={active && tab.id === activeId}>
              <QueryPane
                id={tab.id}
                text={tab.text}
                onTextChange={(text) => dispatch({ type: "setText", id: tab.id, text })}
                schema={schema}
                onRun={recordRun}
                onStatus={setStatus}
                editorHandle={(handle) => {
                  if (handle) editors.current.set(tab.id, handle);
                  else editors.current.delete(tab.id);
                }}
              />
            </ActiveContext.Provider>
          </div>
        ))}
      </div>
    </div>
  );
}
