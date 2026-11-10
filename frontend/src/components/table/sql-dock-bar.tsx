import { PanelBottomClose, PanelBottomOpen } from "lucide-react";
import { cn } from "@/lib/utils";

// Clicks on these keep their own behavior instead of toggling the dock.
const INTERACTIVE = "button, a, input, select, textarea, label, [role='button'], [contenteditable='true']";

/** The SQL dock's header bar, shared by the expanded panel and the collapsed strip. Clicking any
 *  non-interactive part of the bar toggles the dock; the toggle button pinned at the right end
 *  is the keyboard-focusable control. */
export function SqlDockBar({
  expanded,
  onToggle,
  left,
  right,
  className,
}: {
  expanded: boolean;
  onToggle: () => void;
  left: React.ReactNode;
  /** Controls placed just before the toggle button. */
  right?: React.ReactNode;
  className?: string;
}) {
  const Icon = expanded ? PanelBottomClose : PanelBottomOpen;
  return (
    <div
      onClick={(e) => {
        if (e.target instanceof Element && e.target.closest(INTERACTIVE)) return;
        onToggle();
      }}
      className={cn(
        "flex min-h-10 shrink-0 cursor-pointer flex-wrap items-center gap-2 px-4 py-1.5 transition-colors select-none hover:bg-muted/40 md:px-6",
        className,
      )}
    >
      {left}
      <div className="ml-auto flex min-w-0 items-center gap-1.5">
        {right}
        <button
          type="button"
          onClick={onToggle}
          aria-expanded={expanded}
          aria-label={expanded ? "Collapse SQL panel" : "Expand SQL panel"}
          title={expanded ? "Collapse" : "Expand"}
          className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-accent-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
        >
          <Icon className="h-4 w-4" />
        </button>
      </div>
    </div>
  );
}
