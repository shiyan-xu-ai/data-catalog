import { useEffect, useState } from "react";
import { Link, useParams, useRouterState } from "@tanstack/react-router";
import { FolderTree, PanelLeftClose, Search, TerminalSquare } from "lucide-react";
import { ID_DELIMITER, splitId } from "@/lib/ids";
import { useStoredState } from "@/lib/preferences";
import { cn, toggled } from "@/lib/utils";
import { GlobalSearch } from "@/components/global-search";
import { NamespaceTree } from "@/components/namespace-tree";
import { PreferenceControls } from "@/components/preference-controls";

export function useCurrentLocation() {
  const { nsId, tableId } = useParams({ strict: false });
  const isQuery = useRouterState({ select: (s) => s.matches.some((m) => m.routeId === "/query") });
  const activeId = tableId ?? nsId ?? null;
  const segments = activeId ? splitId(activeId) : [];
  return { activeId, segments, isTable: tableId != null, isQuery };
}

export function AppSidebar({
  className,
  onNavigate,
  collapsible = false,
  rail = false,
}: {
  className?: string;
  onNavigate?: () => void;
  /** Desktop only: allow collapsing to an icon rail (persisted). */
  collapsible?: boolean;
  /** Desktop only: show the icon rail on the current page until the user expands it. */
  rail?: boolean;
}) {
  const { activeId, segments, isTable, isQuery } = useCurrentLocation();
  const [storedCollapsed, setStoredCollapsed] = useStoredState("nav-collapsed", false);
  // On a page with `rail`, the nav shows as its rail until the user expands it there. That
  // choice lives only until they leave the page and never touches the stored preference.
  const [railOpened, setRailOpened] = useState(false);
  useEffect(() => {
    if (!rail) setRailOpened(false);
  }, [rail]);
  const collapsed = collapsible && (rail ? !railOpened : storedCollapsed);
  const expand = () => (rail ? setRailOpened(true) : setStoredCollapsed(false));
  const collapse = () => (rail ? setRailOpened(false) : setStoredCollapsed(true));
  // Set when the rail's search button (or ⌘K) expands the nav, so the search box takes focus.
  const [focusSearch, setFocusSearch] = useState(false);
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set());

  // Reveal the current location: expand every enclosing namespace (and the namespace itself).
  useEffect(() => {
    const chain = isTable ? segments.slice(0, -1) : segments;
    if (chain.length === 0) return;
    setExpanded((prev) => {
      const ids = chain.map((_, i) => chain.slice(0, i + 1).join(ID_DELIMITER));
      if (ids.every((id) => prev.has(id))) return prev;
      return new Set([...prev, ...ids]);
    });
    // `activeId` determines `segments` and `isTable`.
  }, [activeId]);

  function toggle(id: string) {
    setExpanded((prev) => toggled(prev, id));
  }

  function expandToSearch() {
    setFocusSearch(true);
    expand();
  }

  // While collapsed the search box is not mounted, so the rail handles its shortcut.
  useEffect(() => {
    if (!collapsed) return;
    function onKey(e: KeyboardEvent) {
      if (e.key === "k" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        expandToSearch();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  if (collapsed) {
    const railButton =
      "flex h-8 w-8 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-accent-foreground";
    // A click anywhere on the rail expands it (links still navigate); theme and timezone
    // controls stop the click so they only change their setting.
    return (
      <aside
        onClick={expand}
        title="Expand navigation"
        className={cn("flex h-full w-14 shrink-0 cursor-pointer flex-col items-center border-r bg-sidebar hover:bg-accent/40", className)}
      >
        <div className="flex h-14 w-full shrink-0 items-center justify-center border-b">
          <Link to="/namespaces" aria-label="Data Catalog home" title={`Data Catalog v${__APP_VERSION__}`}>
            <img src="/logo-light.png" alt="" className="h-5 w-5 object-cover object-left dark:hidden" />
            <img src="/logo-dark.png" alt="" aria-hidden="true" className="hidden h-5 w-5 object-cover object-left dark:block" />
          </Link>
        </div>
        <div className="flex flex-col items-center gap-1 py-3">
          <button type="button" className={railButton} onClick={expandToSearch} aria-label="Search" title="Search (⌘K)">
            <Search className="h-4 w-4" />
          </button>
          <Link to="/namespaces" className={railButton} aria-label="Namespaces" title="Namespaces">
            <FolderTree className="h-4 w-4" />
          </Link>
          {/* The click must not also expand the rail: elsewhere that would clear the stored
              collapsed preference just to open a page that shows the rail regardless. */}
          <Link
            to="/query"
            className={cn(railButton, isQuery && "bg-accent text-accent-foreground")}
            aria-label="Query"
            aria-current={isQuery ? "page" : undefined}
            title="Query"
            onClick={(e) => e.stopPropagation()}
          >
            <TerminalSquare className="h-4 w-4" />
          </Link>
        </div>
        <div
          className="mt-auto flex cursor-default flex-col items-center gap-1 border-t py-3"
          onClick={(e) => e.stopPropagation()}
        >
          <PreferenceControls compact />
        </div>
      </aside>
    );
  }

  return (
    <aside className={cn("flex h-full w-72 shrink-0 flex-col border-r bg-sidebar", className)}>
      <div className="flex h-14 shrink-0 items-center gap-2 border-b pr-2 pl-4">
        <Link to="/namespaces" onClick={onNavigate} className="flex min-w-0 items-center gap-2 font-semibold tracking-tight">
          <img src="/logo-light.png" alt="Applied Intuition" className="h-5 w-auto dark:hidden" />
          <img src="/logo-dark.png" alt="" aria-hidden="true" className="hidden h-5 w-auto dark:block" />
          <span className="truncate text-sm">Data Catalog</span>
          <span className="shrink-0 self-end pb-0.5 font-mono text-[10px] font-normal text-muted-foreground">
            v{__APP_VERSION__}
          </span>
        </Link>
        {collapsible && (
          <button
            type="button"
            onClick={collapse}
            aria-label="Collapse navigation"
            title="Collapse navigation"
            className="ml-auto flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-accent-foreground"
          >
            <PanelLeftClose className="h-4 w-4" />
          </button>
        )}
      </div>
      <div className="shrink-0 p-3">
        <GlobalSearch
          onNavigate={onNavigate}
          autoFocus={focusSearch}
          onFocusHandled={() => setFocusSearch(false)}
        />
      </div>
      <div className="shrink-0 px-2 pb-1">
        <Link
          to="/query"
          onClick={onNavigate}
          aria-current={isQuery ? "page" : undefined}
          className={cn(
            "flex h-8 items-center gap-2 rounded-md px-2 text-[13px] font-medium text-foreground/80 hover:bg-accent hover:text-accent-foreground",
            isQuery && "bg-accent text-accent-foreground",
          )}
        >
          <TerminalSquare className="h-4 w-4 text-muted-foreground" />
          Query
        </Link>
      </div>
      <div className="flex items-center justify-between px-4 pt-1 pb-1.5">
        <Link
          to="/namespaces"
          onClick={onNavigate}
          className="text-[11px] font-medium tracking-wider text-muted-foreground uppercase hover:text-foreground"
        >
          Namespaces
        </Link>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-4">
        <NamespaceTree expanded={expanded} toggle={toggle} activeId={activeId} onNavigate={onNavigate} />
      </div>
      <div className="shrink-0 border-t px-4 py-3">
        <PreferenceControls />
      </div>
    </aside>
  );
}
