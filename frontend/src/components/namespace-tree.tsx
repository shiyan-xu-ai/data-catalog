import { useEffect, useRef } from "react";
import { Link } from "@tanstack/react-router";
import { ChevronRight, Folder, FolderOpen, Loader2, Table2 } from "lucide-react";
import { useChildNamespaces, useNamespaceTables, type PagedNames } from "@/lib/catalog-queries";
import { ID_DELIMITER } from "@/lib/ids";
import { cn } from "@/lib/utils";

interface TreeState {
  expanded: ReadonlySet<string>;
  toggle: (id: string) => void;
  /** Joined id of the current namespace or table page, if any. */
  activeId: string | null;
  onNavigate?: () => void;
}

const rowClass =
  "group flex h-7 w-full min-w-0 items-center gap-1.5 rounded-md pr-2 text-[13px] text-foreground/80 hover:bg-accent hover:text-accent-foreground";
const activeClass = "bg-accent font-medium text-accent-foreground";
// Leaves and status rows align their icon with sibling namespaces' folder icon (past the chevron).
const leafIndent = (depth: number) => ({ paddingLeft: `${depth * 12 + 4 + 26}px` });

function useScrollIntoView(active: boolean) {
  const ref = useRef<HTMLAnchorElement>(null);
  useEffect(() => {
    if (active) ref.current?.scrollIntoView({ block: "nearest" });
  }, [active]);
  return ref;
}

function TableLeaf({ segments, depth, state }: { segments: string[]; depth: number; state: TreeState }) {
  const id = segments.join(ID_DELIMITER);
  const active = state.activeId === id;
  const ref = useScrollIntoView(active);
  return (
    <Link
      ref={ref}
      to="/tables/$tableId"
      params={{ tableId: id }}
      onClick={state.onNavigate}
      className={cn(rowClass, active && activeClass)}
      style={leafIndent(depth)}
      title={segments[segments.length - 1]}
    >
      <Table2 className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
      <span className="truncate font-mono text-xs">{segments[segments.length - 1]}</span>
    </Link>
  );
}

function LoadMore({ query, depth }: { query: PagedNames; depth: number }) {
  if (!query.hasNextPage) return null;
  return (
    <button
      type="button"
      className={cn(rowClass, "text-xs text-muted-foreground")}
      style={leafIndent(depth)}
      disabled={query.isFetchingNextPage}
      onClick={() => query.fetchNextPage()}
    >
      {query.isFetchingNextPage ? "Loading…" : "Load more…"}
    </button>
  );
}

function StatusRow({ depth, children, error }: { depth: number; children: React.ReactNode; error?: boolean }) {
  return (
    <div
      className={cn("flex h-7 items-center gap-1.5 text-xs", error ? "text-destructive" : "text-muted-foreground")}
      style={leafIndent(depth)}
    >
      {children}
    </div>
  );
}

/** Children of an expanded namespace: child namespaces first, then its tables as leaves. */
function TreeChildren({ segments, depth, state }: { segments: string[]; depth: number; state: TreeState }) {
  const namespaces = useChildNamespaces(segments);
  const tables = useNamespaceTables(segments);
  if (namespaces.isLoading || tables.isLoading) {
    return (
      <StatusRow depth={depth}>
        <Loader2 className="h-3 w-3 animate-spin" /> Loading…
      </StatusRow>
    );
  }
  const error = namespaces.error ?? tables.error;
  if (error) {
    return (
      <StatusRow depth={depth} error>
        <span className="truncate" title={error.message}>Failed to load</span>
      </StatusRow>
    );
  }
  if (namespaces.names.length === 0 && tables.names.length === 0) {
    return (
      <StatusRow depth={depth}>
        <span className="italic">{segments.length ? "Empty" : "No namespaces yet"}</span>
      </StatusRow>
    );
  }
  return (
    <>
      {namespaces.names.map((name) => (
        <TreeNamespace key={`n:${name}`} segments={[...segments, name]} depth={depth} state={state} />
      ))}
      <LoadMore query={namespaces} depth={depth} />
      {tables.names.map((name) => (
        <TableLeaf key={`t:${name}`} segments={[...segments, name]} depth={depth} state={state} />
      ))}
      <LoadMore query={tables} depth={depth} />
    </>
  );
}

function TreeNamespace({ segments, depth, state }: { segments: string[]; depth: number; state: TreeState }) {
  const id = segments.join(ID_DELIMITER);
  const open = state.expanded.has(id);
  const active = state.activeId === id;
  const ref = useScrollIntoView(active);
  const FolderIcon = open ? FolderOpen : Folder;
  return (
    <>
      {/* A click anywhere on the row toggles it; the chevron only shows the state, and the name
          still opens the namespace page. */}
      <div
        onClick={() => state.toggle(id)}
        className={cn(rowClass, "cursor-pointer", active && activeClass)}
        style={{ paddingLeft: `${depth * 12 + 4}px` }}
      >
        <span aria-hidden className="flex h-5 w-5 shrink-0 items-center justify-center text-muted-foreground">
          <ChevronRight className={cn("h-3.5 w-3.5 transition-transform", open && "rotate-90")} />
        </span>
        <Link
          ref={ref}
          to="/namespaces/$nsId"
          params={{ nsId: id }}
          onClick={state.onNavigate}
          aria-expanded={open}
          className="flex min-w-0 flex-1 items-center gap-1.5"
          title={segments[segments.length - 1]}
        >
          <FolderIcon className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
          <span className="truncate">{segments[segments.length - 1]}</span>
        </Link>
      </div>
      {open && <TreeChildren segments={segments} depth={depth + 1} state={state} />}
    </>
  );
}

/** Lazily expanding namespace tree rooted at the catalog root. */
export function NamespaceTree(props: TreeState) {
  return (
    <nav aria-label="Namespaces" className="flex flex-col gap-px">
      <TreeChildren segments={[]} depth={0} state={props} />
    </nav>
  );
}
