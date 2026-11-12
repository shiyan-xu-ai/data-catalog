import { useState } from "react";
import { ChevronRight } from "lucide-react";
import { formatCount } from "@/lib/format";
import { cn } from "@/lib/utils";

/** Children shown per expanded object or array before "Show more". */
const PAGE = 100;

function Scalar({ value }: { value: unknown }) {
  if (value === null) return <span className="text-muted-foreground">null</span>;
  if (typeof value === "string") {
    return <span className="break-all text-emerald-700 dark:text-emerald-400">&quot;{value}&quot;</span>;
  }
  if (typeof value === "number") return <span className="text-sky-700 tabular-nums dark:text-sky-400">{value}</span>;
  if (typeof value === "boolean") return <span className="text-violet-700 dark:text-violet-400">{String(value)}</span>;
  return <span>{String(value)}</span>;
}

function childEntries(value: object, limit: number): [string, unknown][] {
  if (Array.isArray(value)) return value.slice(0, limit).map((v, i) => [String(i), v]);
  return Object.keys(value)
    .slice(0, limit)
    .map((k) => [k, (value as Record<string, unknown>)[k]]);
}

function Node({ name, value, depth }: { name: string; value: unknown; depth: number }) {
  const [open, setOpen] = useState(depth === 0);
  const [shown, setShown] = useState(PAGE);
  const label = <span className="text-foreground/80">{name}</span>;
  if (value === null || typeof value !== "object") {
    return (
      <div className="pl-4">
        {label}: <Scalar value={value} />
      </div>
    );
  }
  const isArray = Array.isArray(value);
  const count = isArray ? value.length : Object.keys(value).length;
  const size = isArray ? `[${formatCount(count)}]` : `{${formatCount(count)}}`;
  return (
    <div>
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
        className="flex items-center gap-0.5 rounded-sm text-left hover:bg-accent/60"
      >
        <ChevronRight className={cn("h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform", open && "rotate-90")} />
        {label}
        <span className="ml-1 text-muted-foreground">{size}</span>
      </button>
      {open && (
        <div className="ml-1.5 border-l pl-2">
          {childEntries(value, shown).map(([k, v]) => (
            <Node key={k} name={k} value={v} depth={depth + 1} />
          ))}
          {count > shown && (
            <button
              type="button"
              onClick={() => setShown(shown + PAGE)}
              className="pl-4 text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
            >
              Show {formatCount(Math.min(PAGE, count - shown))} more of {formatCount(count - shown)}
            </button>
          )}
        </div>
      )}
    </div>
  );
}

/** Collapsible JSON view: the root starts open, everything below collapsed; long objects and
 *  arrays render their children a page at a time. */
export function JsonTree({ name, value }: { name: string; value: unknown }) {
  return (
    <div className="font-mono text-xs leading-5">
      <Node name={name} value={value} depth={0} />
    </div>
  );
}
