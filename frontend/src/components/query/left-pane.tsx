import { useState } from "react";
import { PanelLeftClose, PanelLeftOpen, X } from "lucide-react";
import type { HistoryEntry } from "@/lib/query-history";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { HistoryList } from "./history-list";
import { TableBrowser } from "./table-browser";

type Section = "tables" | "history";

const SECTIONS: readonly (readonly [Section, string])[] = [
  ["tables", "Tables"],
  ["history", "History"],
];

/** The Query page's left pane, switching between the table browser and the run history. Docked,
 *  it is 16rem wide and collapses to a narrow strip; as a drawer (`docked` false) it fills the
 *  height and `onClose` dismisses it. Both sections stay mounted so the tree keeps its state. */
export function LeftPane({
  docked,
  collapsed,
  onCollapsedChange,
  onClose,
  browser,
  history,
}: {
  docked: boolean;
  collapsed: boolean;
  onCollapsedChange: (collapsed: boolean) => void;
  onClose: () => void;
  browser: React.ComponentProps<typeof TableBrowser>;
  history: { entries: readonly HistoryEntry[]; onSelect: (sql: string) => void };
}) {
  const [section, setSection] = useState<Section>("tables");

  if (docked && collapsed) {
    return (
      <div className="flex w-9 shrink-0 flex-col items-center border-r bg-sidebar py-2">
        <Button
          variant="ghost"
          size="icon"
          className="h-7 w-7"
          aria-label="Show tables and history"
          title="Show tables and history"
          onClick={() => onCollapsedChange(false)}
        >
          <PanelLeftOpen />
        </Button>
      </div>
    );
  }

  return (
    <aside
      aria-label="Tables and history"
      className={cn("flex min-h-0 flex-col bg-sidebar", docked ? "w-64 shrink-0 border-r" : "h-full w-[min(20rem,85vw)] shadow-xl")}
    >
      <div className="flex h-10 shrink-0 items-center gap-1 border-b px-2">
        <div role="tablist" aria-label="Left pane section" className="flex gap-1">
          {SECTIONS.map(([value, label]) => (
            <button
              key={value}
              type="button"
              role="tab"
              aria-selected={section === value}
              onClick={() => setSection(value)}
              className={cn(
                "rounded-md px-2.5 py-1 text-xs transition-colors",
                section === value ? "bg-accent font-medium text-foreground" : "text-muted-foreground hover:bg-accent/60",
              )}
            >
              {label}
            </button>
          ))}
        </div>
        {docked ? (
          <Button
            variant="ghost"
            size="icon"
            className="ml-auto h-7 w-7"
            aria-label="Hide tables and history"
            title="Hide tables and history"
            onClick={() => onCollapsedChange(true)}
          >
            <PanelLeftClose />
          </Button>
        ) : (
          <Button variant="ghost" size="icon" className="ml-auto h-7 w-7" aria-label="Close tables and history" onClick={onClose}>
            <X />
          </Button>
        )}
      </div>
      <div className={cn("min-h-0 flex-1 flex-col", section === "tables" ? "flex" : "hidden")}>
        <TableBrowser {...browser} />
      </div>
      <div className={cn("min-h-0 flex-1 flex-col", section === "history" ? "flex" : "hidden")}>
        <HistoryList {...history} />
      </div>
    </aside>
  );
}
