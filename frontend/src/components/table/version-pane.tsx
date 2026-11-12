import { useEffect, useRef, useState } from "react";
import { X } from "lucide-react";
import { useVersionFiles } from "@/lib/catalog-queries";
import { formatBytes, formatCount } from "@/lib/format";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { CopyButton } from "@/components/copy-button";
import { ErrorState } from "@/components/error-state";
import { JsonTree } from "@/components/json-tree";

export type PaneView = "details" | "source";

/** One `label: value` fact in a pane section. */
export function Fact({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-1 text-sm">
      <span className="shrink-0 text-xs text-muted-foreground">{label}</span>
      <span className="min-w-0">{children}</span>
    </div>
  );
}

export function Mono({ children }: { children: React.ReactNode }) {
  return <code className="rounded bg-muted px-1 py-0.5 font-mono text-xs wrap-break-word">{children}</code>;
}

/** One labeled section of a version pane. */
export function PaneSection({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="space-y-1.5 border-t px-4 py-3">
      <h4 className="text-xs font-medium tracking-wide text-muted-foreground uppercase">{title}</h4>
      {children}
    </section>
  );
}

const SUMMARY_FIELDS: [key: string, label: string, bytes?: boolean][] = [
  ["total_rows", "Rows"],
  ["total_files_size", "Data size", true],
  ["total_fragments", "Fragments"],
  ["total_data_files", "Data files"],
  ["total_deletion_files", "Deletion files"],
  ["total_deletion_file_rows", "Deleted rows"],
];

/** The manifest summary totals of one version. */
export function SummaryTotals({ summary }: { summary: Record<string, string> }) {
  return (
    <dl className="grid grid-cols-2 gap-x-4 gap-y-1.5">
      {SUMMARY_FIELDS.map(([key, label, bytes]) => {
        const raw = summary[key];
        const n = raw == null || raw === "" ? null : Number(raw);
        return (
          <div key={key} className="min-w-0">
            <dt className="text-xs text-muted-foreground">{label}</dt>
            <dd className="text-sm tabular-nums">{bytes ? formatBytes(n) : formatCount(n)}</dd>
          </div>
        );
      })}
    </dl>
  );
}

/** The files that record `version`, each protobuf section as a JSON tree. */
function VersionSource({ segments, version }: { segments: readonly string[]; version: number }) {
  const files = useVersionFiles(segments, version);
  if (files.error) {
    return (
      <div className="p-4">
        <ErrorState message={files.error.message} />
      </div>
    );
  }
  if (files.isPending) {
    return (
      <div className="space-y-2 p-4">
        <Skeleton className="h-4 w-2/3" />
        <Skeleton className="h-40 w-full" />
      </div>
    );
  }
  return (
    <>
      <p className="px-4 pb-3 text-xs text-muted-foreground">
        Protobuf decoded to JSON. Byte fields over 64 bytes show only their length.
      </p>
      {files.data.files.map((f) => (
        <section key={f.path} className="space-y-2 border-t px-4 py-3">
          <div className="flex items-center gap-1">
            <code className="min-w-0 font-mono text-xs break-all">{f.path}</code>
            <CopyButton value={f.path} label="Copy path" />
            <span className="ml-auto shrink-0 text-xs text-muted-foreground tabular-nums">{formatBytes(f.size_bytes)}</span>
          </div>
          {f.sections.map((s) => (
            <div key={`${s.name}@${s.offset}`} className="rounded-md border">
              <div className="flex items-center gap-2 border-b bg-muted/50 py-1 pr-1 pl-2 text-xs">
                <span className="font-medium">{s.name}</span>
                {s.message && <span className="truncate font-mono text-muted-foreground">{s.message}</span>}
                <span className="ml-auto shrink-0 text-muted-foreground tabular-nums">@ {formatCount(s.offset)}</span>
                {s.content !== undefined && (
                  <CopyButton value={() => JSON.stringify(s.content, null, 2)} label="Copy JSON" />
                )}
              </div>
              <div className="overflow-x-auto p-2">
                {s.error ? (
                  <span className="text-xs text-destructive">{s.error}</span>
                ) : (
                  <JsonTree name={s.message?.split(".").pop() ?? s.name} value={s.content} />
                )}
              </div>
            </div>
          ))}
        </section>
      ))}
    </>
  );
}

const VIEWS: [PaneView, string][] = [
  ["details", "Details"],
  ["source", "Source files"],
];

/** Detail pane for one version: formatted `children`, or the decoded files behind it. Sticky
 *  beside the list on wide screens (`top` px below the scroll area's top), an overlay on narrow
 *  ones; the source view is wider. */
export function VersionPane({
  label,
  header,
  segments,
  version,
  view,
  onView,
  onClose,
  top = 0,
  children,
}: {
  label: string;
  header: React.ReactNode;
  segments: readonly string[];
  version: number;
  view: PaneView;
  onView: (view: PaneView) => void;
  onClose: () => void;
  top?: number;
  children: React.ReactNode;
}) {
  const ref = useRef<HTMLElement>(null);
  const [maxHeight, setMaxHeight] = useState<number>();
  // Fit the sticky pane to the page's scroll area, which shrinks as the SQL dock grows.
  useEffect(() => {
    const main = ref.current?.closest("main");
    if (!main) return;
    const observer = new ResizeObserver(() => setMaxHeight(main.clientHeight - 32 - top));
    observer.observe(main);
    return () => observer.disconnect();
  }, [top]);

  return (
    <>
      <div aria-hidden className="fixed inset-0 z-40 bg-black/30 xl:hidden" onClick={onClose} />
      <aside
        ref={ref}
        aria-label={label}
        style={
          {
            "--pane-max": maxHeight ? `${maxHeight}px` : "100%",
            "--pane-top": `${top + 16}px`,
          } as React.CSSProperties
        }
        className={cn(
          "fixed inset-y-0 right-0 z-50 flex w-full flex-col overflow-hidden border-l bg-background shadow-xl xl:sticky xl:top-(--pane-top) xl:z-10 xl:max-h-(--pane-max) xl:shrink-0 xl:self-start xl:rounded-lg xl:border xl:shadow-sm",
          view === "source" ? "max-w-[720px] xl:w-[720px]" : "max-w-[420px] xl:w-[420px]",
        )}
      >
        <div className="flex items-start gap-3 px-4 pt-3 pb-2">
          <div className="min-w-0 flex-1">{header}</div>
          <Button variant="ghost" size="icon" className="h-7 w-7" onClick={onClose} aria-label="Close details">
            <X />
          </Button>
        </div>
        <div role="tablist" aria-label="View" className="flex gap-1 px-4 pb-3">
          {VIEWS.map(([v, name]) => (
            <button
              key={v}
              type="button"
              role="tab"
              aria-selected={view === v}
              onClick={() => onView(v)}
              className={cn(
                "rounded-md px-2.5 py-1 text-xs transition-colors",
                view === v ? "bg-accent font-medium text-foreground" : "text-muted-foreground hover:bg-accent/60",
              )}
            >
              {name}
            </button>
          ))}
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto">
          {view === "details" ? children : <VersionSource segments={segments} version={version} />}
        </div>
      </aside>
    </>
  );
}
