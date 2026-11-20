import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { ActiveContext } from "@/lib/active";
import { useAllTableIds, useTableColumns } from "@/lib/catalog-queries";
import { joinId, splitId } from "@/lib/ids";
import { addHistory, loadHistory, saveHistory } from "@/lib/query-history";
import { initialTabs, tabsReducer } from "@/lib/query-tabs";
import { completionSchema, findTablePaths, sampleQuery } from "@/lib/sql";
import { registerSqlInsert } from "@/lib/sql-insert";
import { cn } from "@/lib/utils";
import type { SqlEditorHandle } from "@/components/sql-editor";
import { QueryPane, type RunStatus } from "./query-pane";
import { QueryTabStrip } from "./query-tab-strip";

const COLUMN_LOAD_DEBOUNCE_MS = 400;

function useDebounced<T>(value: T, delayMs: number): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const id = setTimeout(() => setDebounced(value), delayMs);
    return () => clearTimeout(id);
  }, [value, delayMs]);
  return debounced;
}

/** The Query page, the main area in SQL mode: query tabs, an editor, and results. The sidebar's
 *  tree picks tables into the active editor. Every tab stays mounted (hidden when inactive) so its
 *  text, results, and running query survive switching tabs, and the root keeps the whole page
 *  mounted once visited.
 *
 *  `table` is the `table` search parameter, a joined table id: it opens one tab pre-filled with a
 *  sample query of that table (the initial tab when the page mounts with it) and is then cleared
 *  from the URL. */
export function QueryPage({ active, table }: { active: boolean; table: string | undefined }) {
  const navigate = useNavigate();
  const [state, dispatch] = useReducer(tabsReducer, table, (t) => initialTabs(t ? sampleQuery(splitId(t)) : ""));
  const { tabs, activeId } = state;
  const [statuses, setStatuses] = useState<Record<number, RunStatus>>({});
  const [history, setHistory] = useState(loadHistory);
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
  const activeText = tabs.find((t) => t.id === activeId)?.text ?? "";
  const settledText = useDebounced(activeText, COLUMN_LOAD_DEBOUNCE_MS);
  const named = useMemo(() => findTablePaths(settledText, tableIds.data ?? []).map(joinId), [settledText, tableIds.data]);
  const columns = useTableColumns(named);
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

  useEffect(() => registerSqlInsert((text) => editors.current.get(activeId)?.insert(text)), [activeId]);

  return (
    <div className="flex h-full min-h-0 min-w-0 flex-col bg-card">
      <div className="flex shrink-0 items-center border-b bg-muted/30">
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
              history={history}
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
  );
}
