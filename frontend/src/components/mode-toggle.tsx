import { Compass, TerminalSquare } from "lucide-react";
import { useNavigate, useRouter } from "@tanstack/react-router";
import { lastExploreLocation } from "@/lib/explore-location";
import { cn } from "@/lib/utils";

export type Mode = "explore" | "sql";

const MODES: { value: Mode; label: string; icon: typeof Compass }[] = [
  { value: "explore", label: "Explore", icon: Compass },
  { value: "sql", label: "SQL", icon: TerminalSquare },
];

/** Switches between the catalog pages and the SQL editor; the route is the source of truth. SQL
 *  opens `/query`, Explore returns to the last page seen outside it. */
function useSwitchMode() {
  const navigate = useNavigate();
  const router = useRouter();
  return (mode: Mode) => {
    if (mode === "sql") navigate({ to: "/query" });
    else router.history.push(lastExploreLocation());
  };
}

/** Segmented Explore | SQL control for the expanded sidebar. */
export function ModeToggle({ mode }: { mode: Mode }) {
  const switchMode = useSwitchMode();
  return (
    <div role="radiogroup" aria-label="Mode" className="grid grid-cols-2 rounded-md border bg-background p-0.5">
      {MODES.map(({ value, label, icon: Icon }) => {
        const active = value === mode;
        return (
          <button
            key={value}
            type="button"
            role="radio"
            aria-checked={active}
            onClick={() => !active && switchMode(value)}
            className={cn(
              "flex h-7 items-center justify-center gap-1.5 rounded-sm text-xs transition-colors",
              active ? "bg-accent font-medium text-accent-foreground shadow-xs" : "text-muted-foreground hover:text-foreground",
            )}
          >
            <Icon className="h-3.5 w-3.5" />
            {label}
          </button>
        );
      })}
    </div>
  );
}

/** The collapsed rail's control: one button showing the current mode that switches to the
 *  other. It stops the click so the rail itself does not also expand. */
export function ModeToggleRail({ mode, className }: { mode: Mode; className?: string }) {
  const switchMode = useSwitchMode();
  const next = mode === "sql" ? "explore" : "sql";
  const current = MODES.find((m) => m.value === mode)!;
  const target = MODES.find((m) => m.value === next)!;
  return (
    <button
      type="button"
      className={className}
      aria-label={`${current.label} mode. Switch to ${target.label}`}
      title={`${current.label} mode. Switch to ${target.label}`}
      onClick={(e) => {
        e.stopPropagation();
        switchMode(next);
      }}
    >
      <current.icon className="h-4 w-4" />
    </button>
  );
}
