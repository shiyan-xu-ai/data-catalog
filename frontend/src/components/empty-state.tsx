import type { LucideIcon } from "lucide-react";
import { cn } from "@/lib/utils";

/** Explicit "nothing here" message, so empty sections never silently disappear. */
export function EmptyState({
  icon: Icon,
  title,
  children,
  className,
}: {
  icon?: LucideIcon;
  title?: string;
  children?: React.ReactNode;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "flex flex-col items-center justify-center gap-1.5 rounded-md border border-dashed px-4 py-8 text-center",
        className,
      )}
    >
      {Icon && <Icon className="mb-1 h-5 w-5 text-muted-foreground/70" />}
      {title && <p className="text-sm font-medium">{title}</p>}
      {children && <div className="text-sm text-muted-foreground">{children}</div>}
    </div>
  );
}

/** Placeholder for a missing value in a table cell or fact. */
export function Dash() {
  return <span className="text-muted-foreground/60">—</span>;
}
