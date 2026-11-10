import { useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { Folder, Loader2, Search, Table2 } from "lucide-react";
import { useAllNamespaces, useAllTableIds } from "@/lib/catalog-queries";
import { ID_DELIMITER, displayName } from "@/lib/ids";
import { buildSearchIndex, searchCatalog, type SearchEntry } from "@/lib/search";
import { cn } from "@/lib/utils";

const MAX_RESULTS = 20;

/** Name search over every table (from the list-all route) and every namespace (enumerated
 *  from the root on first focus). ⌘K / Ctrl+K or `/` focuses it. */
export function GlobalSearch({
  onNavigate,
  autoFocus = false,
  onFocusHandled,
}: {
  onNavigate?: () => void;
  /** Focus the box once (e.g. after the collapsed rail's search button expanded the nav). */
  autoFocus?: boolean;
  onFocusHandled?: () => void;
}) {
  const navigate = useNavigate();
  const inputRef = useRef<HTMLInputElement>(null);
  const [query, setQuery] = useState("");
  const [open, setOpen] = useState(false);
  const [activated, setActivated] = useState(false);
  const [cursor, setCursor] = useState(0);
  const tables = useAllTableIds(activated);
  const namespaces = useAllNamespaces(activated);
  const loading = tables.isLoading || namespaces.isLoading;
  const loadError = tables.error ?? namespaces.error;

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      const target = e.target as HTMLElement | null;
      const typing = target && (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable);
      if ((e.key === "k" && (e.metaKey || e.ctrlKey)) || (e.key === "/" && !typing)) {
        e.preventDefault();
        inputRef.current?.focus();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  useEffect(() => {
    if (!autoFocus) return;
    inputRef.current?.focus();
    onFocusHandled?.();
  }, [autoFocus, onFocusHandled]);

  const index = useMemo(
    () => buildSearchIndex(tables.data ?? [], namespaces.data ?? []),
    [tables.data, namespaces.data],
  );
  const results = useMemo(() => searchCatalog(index, query, MAX_RESULTS), [index, query]);

  useEffect(() => setCursor(0), [query]);

  function go(entry: SearchEntry) {
    const id = entry.segments.join(ID_DELIMITER);
    if (entry.kind === "table") navigate({ to: "/tables/$tableId", params: { tableId: id } });
    else navigate({ to: "/namespaces/$nsId", params: { nsId: id } });
    setQuery("");
    setOpen(false);
    inputRef.current?.blur();
    onNavigate?.();
  }

  function onKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setCursor((c) => Math.min(c + 1, results.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setCursor((c) => Math.max(c - 1, 0));
    } else if (e.key === "Enter" && results[cursor]) {
      e.preventDefault();
      go(results[cursor]);
    } else if (e.key === "Escape") {
      setOpen(false);
      inputRef.current?.blur();
    }
  }

  const showPanel = open && query.trim().length > 0;

  return (
    <div className="relative">
      <Search className="pointer-events-none absolute top-1/2 left-2.5 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
      <input
        ref={inputRef}
        value={query}
        onChange={(e) => {
          setQuery(e.target.value);
          setOpen(true);
        }}
        onFocus={() => {
          setActivated(true);
          setOpen(true);
        }}
        onBlur={() => setOpen(false)}
        onKeyDown={onKeyDown}
        placeholder="Search namespaces and tables"
        aria-label="Search namespaces and tables"
        role="combobox"
        aria-expanded={showPanel}
        aria-controls="global-search-results"
        className="h-8 w-full rounded-md border border-input bg-background pr-10 pl-8 text-[13px] shadow-xs placeholder:text-muted-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
      />
      <kbd className="pointer-events-none absolute top-1/2 right-2 -translate-y-1/2 rounded border bg-muted px-1 font-mono text-[10px] text-muted-foreground">
        ⌘K
      </kbd>
      {showPanel && (
        <div
          id="global-search-results"
          role="listbox"
          className="absolute top-full right-0 left-0 z-50 mt-1 max-h-96 overflow-y-auto rounded-lg border bg-popover p-1 text-popover-foreground shadow-lg"
        >
          {loading && results.length === 0 ? (
            <div className="flex items-center gap-2 px-2 py-3 text-xs text-muted-foreground">
              <Loader2 className="h-3 w-3 animate-spin" /> Loading catalog index…
            </div>
          ) : results.length === 0 ? (
            <div className="px-2 py-3 text-xs text-muted-foreground">
              {loadError ? `Search index failed to load: ${loadError.message}` : "No namespaces or tables match."}
            </div>
          ) : (
            results.map((entry, i) => {
              const Icon = entry.kind === "table" ? Table2 : Folder;
              const name = entry.segments[entry.segments.length - 1];
              const parent = displayName(entry.segments.slice(0, -1));
              return (
                <div
                  key={`${entry.kind}:${entry.segments.join(ID_DELIMITER)}`}
                  role="option"
                  aria-selected={i === cursor}
                  // Keep focus in the input so blur does not close the panel before the click.
                  onMouseDown={(e) => e.preventDefault()}
                  onMouseEnter={() => setCursor(i)}
                  onClick={() => go(entry)}
                  className={cn(
                    "flex cursor-pointer items-center gap-2 rounded-md px-2 py-1.5",
                    i === cursor && "bg-accent text-accent-foreground",
                  )}
                >
                  <Icon className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                  <div className="min-w-0">
                    <div className="truncate font-mono text-xs">{name}</div>
                    {parent && <div className="truncate text-[11px] text-muted-foreground">{parent}</div>}
                  </div>
                </div>
              );
            })
          )}
        </div>
      )}
    </div>
  );
}
