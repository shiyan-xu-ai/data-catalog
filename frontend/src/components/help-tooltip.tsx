import { useRef, useState } from "react";
import { CircleHelp } from "lucide-react";
import { cn } from "@/lib/utils";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";

/** A (?) button explaining the figure or control next to it. It is a real button, so the
 *  tooltip opens on hover and on keyboard focus; the content is portaled above any clipping
 *  container. `label` names the button for screen readers. */
export function HelpTooltip({
  label,
  children,
  className,
}: {
  label: string;
  children: React.ReactNode;
  className?: string;
}) {
  const [open, setOpen] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  const escaped = useRef(false);

  function onOpenChange(next: boolean) {
    // Tabbing to an off-screen trigger scrolls it into view, and Radix closes a tooltip on any
    // scroll of its trigger's container; keep a keyboard-opened tooltip open while the trigger
    // keeps focus (Escape and blur still close it).
    const keyboardFocused = trigger.current === document.activeElement && trigger.current?.matches(":focus-visible");
    if (!next && keyboardFocused && !escaped.current) return;
    escaped.current = false;
    setOpen(next);
  }

  return (
    <Tooltip open={open} onOpenChange={onOpenChange}>
      <TooltipTrigger asChild>
        <button
          ref={trigger}
          type="button"
          aria-label={label}
          className={cn(
            "inline-flex h-4 w-4 shrink-0 items-center justify-center rounded-full text-muted-foreground/80 hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring",
            className,
          )}
        >
          <CircleHelp className="h-3.5 w-3.5" />
        </button>
      </TooltipTrigger>
      <TooltipContent
        side="top"
        onEscapeKeyDown={() => {
          escaped.current = true;
        }}
        className="max-w-xs text-left text-xs leading-relaxed font-normal whitespace-normal"
      >
        {children}
      </TooltipContent>
    </Tooltip>
  );
}
