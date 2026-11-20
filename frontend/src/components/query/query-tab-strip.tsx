import { useEffect, useRef } from "react";
import { Loader2, Plus, X } from "lucide-react";
import { tabTitle, type QueryTab } from "@/lib/query-tabs";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import type { RunStatus } from "./query-pane";

/** Query tabs in a horizontally scrollable tab list, with a new-tab button after it. A running
 *  tab shows a spinner and one whose last run failed an error dot. Closing a tab also stops
 *  waiting for its running query. */
export function QueryTabStrip({
  tabs,
  activeId,
  statuses,
  onActivate,
  onClose,
  onNew,
}: {
  tabs: readonly QueryTab[];
  activeId: number;
  statuses: Readonly<Record<number, RunStatus>>;
  onActivate: (id: number) => void;
  onClose: (id: number) => void;
  onNew: () => void;
}) {
  const buttons = useRef(new Map<number, HTMLButtonElement>());

  useEffect(() => {
    buttons.current.get(activeId)?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, [activeId, tabs.length]);

  function onKeyDown(e: React.KeyboardEvent) {
    const step = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
    if (step === 0) return;
    const index = tabs.findIndex((t) => t.id === activeId);
    const next = tabs[(index + step + tabs.length) % tabs.length];
    e.preventDefault();
    onActivate(next.id);
    buttons.current.get(next.id)?.focus();
  }

  return (
    <div className="flex min-w-0 flex-1 items-center">
      <div role="tablist" aria-label="Query tabs" onKeyDown={onKeyDown} className="flex min-w-0 overflow-x-auto">
        {tabs.map((tab) => {
          const active = tab.id === activeId;
          const status = statuses[tab.id];
          const title = tabTitle(tab);
          return (
            <div
              key={tab.id}
              role="presentation"
              className={cn(
                "group flex h-9 shrink-0 items-center border-r text-xs",
                active ? "bg-background text-foreground" : "bg-muted/40 text-muted-foreground hover:bg-accent/60",
              )}
            >
              <button
                ref={(el) => {
                  if (el) buttons.current.set(tab.id, el);
                  else buttons.current.delete(tab.id);
                }}
                type="button"
                role="tab"
                id={`query-tab-${tab.id}`}
                aria-selected={active}
                aria-controls={`query-panel-${tab.id}`}
                tabIndex={active ? 0 : -1}
                onClick={() => onActivate(tab.id)}
                className={cn(
                  "flex h-full items-center gap-1.5 pr-1 pl-3 whitespace-nowrap focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset",
                  active && "font-medium",
                )}
              >
                {title}
                {status === "running" && <Loader2 aria-label="Running" className="h-3 w-3 animate-spin" />}
                {status === "failed" && (
                  <span role="img" aria-label="Last run failed" className="h-1.5 w-1.5 rounded-full bg-destructive" />
                )}
              </button>
              <button
                type="button"
                aria-label={`Close ${title}`}
                title={status === "running" ? "Stops the running query" : undefined}
                onClick={() => onClose(tab.id)}
                className="mr-1 flex h-5 w-5 items-center justify-center rounded-sm opacity-60 hover:bg-accent hover:opacity-100 focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-hidden"
              >
                <X className="h-3 w-3" />
              </button>
            </div>
          );
        })}
      </div>
      <Button variant="ghost" size="icon" className="mx-1 h-7 w-7 shrink-0" aria-label="New query tab" onClick={onNew}>
        <Plus />
      </Button>
    </div>
  );
}
