import { useMemo, useState } from "react";
import { ChevronRight, Loader2, Search, Table2 } from "lucide-react";
import { useAllTableIds, type TableColumns } from "@/lib/catalog-queries";
import { formatArrowTypeShort } from "@/lib/arrow";
import { ID_DELIMITER, displayName } from "@/lib/ids";
import { searchCatalog, type SearchEntry } from "@/lib/search";
import { quoteIdent, quotePath } from "@/lib/sql";
import { cn } from "@/lib/utils";
import { NamespaceTree, StatusRow, leafIndent, rowClass } from "@/components/namespace-tree";

const MAX_MATCHES = 50;

interface Picker {
  columns: ReadonlyMap<string, TableColumns>;
  open: ReadonlySet<string>;
  toggle: (id: string) => void;
  onInsert: (text: string, tableId?: string) => void;
}

function ColumnRows({ columns, depth, onInsert }: { columns: TableColumns | undefined; depth: number; onInsert: Picker["onInsert"] }) {
  if (!columns || columns.isLoading) {
    return (
      <StatusRow depth={depth}>
        <Loader2 className="h-3 w-3 animate-spin" /> Loading columns…
      </StatusRow>
    );
  }
  if (columns.error) {
    return (
      <StatusRow depth={depth} error>
        <span className="truncate" title={columns.error.message}>Failed to load columns</span>
      </StatusRow>
    );
  }
  if (!columns.fields?.length) {
    return (
      <StatusRow depth={depth}>
        <span className="italic">No columns</span>
      </StatusRow>
    );
  }
  return columns.fields.map((field) => (
    <button
      key={field.name}
      type="button"
      className={cn(rowClass, "text-left")}
      style={leafIndent(depth)}
      title={`Insert ${quoteIdent(field.name)}`}
      onClick={() => onInsert(quoteIdent(field.name))}
    >
      <span className="truncate font-mono text-xs">{field.name}</span>
      <span className="ml-auto shrink-0 pl-2 font-mono text-[10px] text-muted-foreground">{formatArrowTypeShort(field.type)}</span>
    </button>
  ));
}

/** A table row: the chevron lists its columns, the label inserts its quoted full path. */
function TableRow({ segments, depth, picker, showPath }: { segments: string[]; depth: number; picker: Picker; showPath?: boolean }) {
  const id = segments.join(ID_DELIMITER);
  const name = segments[segments.length - 1];
  const open = picker.open.has(id);
  const path = quotePath(segments);
  return (
    <>
      <div className={rowClass} style={{ paddingLeft: `${depth * 12 + 4}px` }}>
        <button
          type="button"
          aria-expanded={open}
          aria-label={`${open ? "Hide" : "Show"} columns of ${name}`}
          onClick={() => picker.toggle(id)}
          className="flex h-5 w-5 shrink-0 items-center justify-center rounded-sm text-muted-foreground hover:bg-background/60"
        >
          <ChevronRight className={cn("h-3.5 w-3.5 transition-transform", open && "rotate-90")} />
        </button>
        <button
          type="button"
          className="flex min-w-0 flex-1 items-center gap-1.5 text-left"
          title={`Insert ${path}`}
          onClick={() => picker.onInsert(path, id)}
        >
          <Table2 className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
          <span className="truncate font-mono text-xs">{showPath ? displayName(segments) : name}</span>
        </button>
      </div>
      {open && <ColumnRows columns={picker.columns.get(id)} depth={depth + 1} onInsert={picker.onInsert} />}
    </>
  );
}

/** The catalog's namespaces and tables for building a query: a lazy tree, or a flat list of full
 *  paths while the name box has text. `columns` holds the loaded schemas by joined table id;
 *  `onWantColumns` asks for one when its table is expanded. */
export function TableBrowser({
  columns,
  onWantColumns,
  onInsert,
}: {
  columns: ReadonlyMap<string, TableColumns>;
  onWantColumns: (id: string) => void;
  /** Insert `text` into the editor; `tableId` names the table whose path it is. */
  onInsert: (text: string, tableId?: string) => void;
}) {
  const tables = useAllTableIds(true);
  const [query, setQuery] = useState("");
  const [namespaces, setNamespaces] = useState<ReadonlySet<string>>(() => new Set());
  const [open, setOpen] = useState<ReadonlySet<string>>(() => new Set());
  const entries = useMemo(
    () => (tables.data ?? []).map((segments): SearchEntry => ({ kind: "table", segments })),
    [tables.data],
  );
  const matches = useMemo(() => searchCatalog(entries, query, MAX_MATCHES), [entries, query]);
  const searching = query.trim().length > 0;

  function toggle<T>(set: ReadonlySet<T>, value: T): Set<T> {
    const next = new Set(set);
    if (!next.delete(value)) next.add(value);
    return next;
  }

  const picker: Picker = {
    columns,
    open,
    toggle: (id) => {
      if (!open.has(id)) onWantColumns(id);
      setOpen(toggle(open, id));
    },
    onInsert,
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="relative shrink-0 p-2">
        <Search className="pointer-events-none absolute top-1/2 left-4.5 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search tables"
          aria-label="Search tables"
          className="h-8 w-full rounded-md border border-input bg-background pr-2 pl-8 text-[13px] shadow-xs placeholder:text-muted-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
        />
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-3">
        {searching ? (
          <div className="flex flex-col gap-px">
            {tables.isLoading ? (
              <StatusRow depth={0}>
                <Loader2 className="h-3 w-3 animate-spin" /> Loading tables…
              </StatusRow>
            ) : tables.error ? (
              <StatusRow depth={0} error>
                <span className="truncate" title={tables.error.message}>Failed to load tables</span>
              </StatusRow>
            ) : matches.length === 0 ? (
              <StatusRow depth={0}>
                <span className="italic">No tables match</span>
              </StatusRow>
            ) : (
              matches.map((m) => (
                <TableRow key={m.segments.join(ID_DELIMITER)} segments={m.segments} depth={0} picker={picker} showPath />
              ))
            )}
          </div>
        ) : (
          <NamespaceTree
            expanded={namespaces}
            toggle={(id) => setNamespaces(toggle(namespaces, id))}
            activeId={null}
            renderTable={(segments, depth) => <TableRow segments={segments} depth={depth} picker={picker} />}
          />
        )}
      </div>
    </div>
  );
}
