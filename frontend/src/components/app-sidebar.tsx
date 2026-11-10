import { useEffect, useState } from "react";
import { Link, useParams } from "@tanstack/react-router";
import { ID_DELIMITER, splitId } from "@/lib/ids";
import { cn } from "@/lib/utils";
import { GlobalSearch } from "@/components/global-search";
import { NamespaceTree } from "@/components/namespace-tree";
import { PreferenceControls } from "@/components/preference-controls";

/** Joined id of the current namespace or table page and the namespace chain to expand. */
export function useCurrentLocation() {
  const { nsId, tableId } = useParams({ strict: false });
  const activeId = tableId ?? nsId ?? null;
  const segments = activeId ? splitId(activeId) : [];
  return { activeId, segments, isTable: tableId != null };
}

export function AppSidebar({ className, onNavigate }: { className?: string; onNavigate?: () => void }) {
  const { activeId, segments, isTable } = useCurrentLocation();
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
    setExpanded((prev) => {
      const next = new Set(prev);
      if (!next.delete(id)) next.add(id);
      return next;
    });
  }

  return (
    <aside className={cn("flex h-full w-72 shrink-0 flex-col border-r bg-sidebar", className)}>
      <div className="flex h-14 shrink-0 items-center gap-2 border-b px-4">
        <Link to="/namespaces" onClick={onNavigate} className="flex items-center gap-2 font-semibold tracking-tight">
          <img src="/logo-light.png" alt="Applied Intuition" className="h-5 w-auto dark:hidden" />
          <img src="/logo-dark.png" alt="" aria-hidden="true" className="hidden h-5 w-auto dark:block" />
          <span className="text-sm">Data Catalog</span>
        </Link>
      </div>
      <div className="shrink-0 p-3">
        <GlobalSearch onNavigate={onNavigate} />
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
