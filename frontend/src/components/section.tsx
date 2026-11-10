import { cn } from "@/lib/utils";
import { HelpTooltip } from "@/components/help-tooltip";

/** Titled surface used for every page section, so spacing and headings stay uniform. */
export function Section({
  title,
  description,
  help,
  actions,
  children,
  className,
  contentClassName,
}: {
  title: string;
  description?: React.ReactNode;
  /** Explanation behind a (?) icon next to the title. */
  help?: React.ReactNode;
  actions?: React.ReactNode;
  children: React.ReactNode;
  className?: string;
  contentClassName?: string;
}) {
  return (
    <section className={cn("rounded-lg border bg-card text-card-foreground shadow-xs", className)}>
      <header className="flex items-start justify-between gap-4 px-4 pt-4 pb-3">
        <div className="min-w-0 space-y-0.5">
          <h2 className="flex items-center gap-1.5 text-sm font-semibold tracking-tight">
            {title}
            {help && <HelpTooltip label={`About ${title}`}>{help}</HelpTooltip>}
          </h2>
          {description && <p className="text-xs text-muted-foreground">{description}</p>}
        </div>
        {actions && <div className="flex shrink-0 items-center gap-2">{actions}</div>}
      </header>
      <div className={cn("px-4 pb-4", contentClassName)}>{children}</div>
    </section>
  );
}
