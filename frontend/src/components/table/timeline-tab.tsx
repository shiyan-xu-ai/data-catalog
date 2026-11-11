import { useEffect, useMemo, useRef, useState } from "react";
import {
  CalendarDays,
  CircleDot,
  Columns3,
  Combine,
  Copy,
  FilePlus2,
  GitCommitVertical,
  ListTree,
  type LucideIcon,
  PenLine,
  Replace,
  RotateCcw,
  Settings2,
  Scissors,
  TableProperties,
  Trash2,
  X,
} from "lucide-react";
import { useTableTimeline } from "@/lib/catalog-queries";
import { formatBytes, formatCount, formatCountOf, formatIsoTime, formatIsoTimestamp, type TimeZonePref } from "@/lib/format";
import { useTimeZonePref } from "@/lib/preferences";
import type { TimelineEvent } from "@/lib/schemas";
import {
  asRecord,
  asString,
  asStrings,
  buildTimelineRows,
  eventSummary,
  groupByDay,
  operationCounts,
  operationFamily,
  operationLabel,
  type DayGroup,
  type OperationFamily,
  type SummaryDelta,
} from "@/lib/timeline";
import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { EmptyState } from "@/components/empty-state";
import { ErrorState } from "@/components/error-state";
import { CopyButton } from "@/components/copy-button";

const ICON: Record<string, LucideIcon> = {
  Append: FilePlus2,
  Overwrite: Replace,
  Merge: Columns3,
  DataReplacement: Replace,
  DataOverlay: Replace,
  Delete: Trash2,
  Update: PenLine,
  Rewrite: Combine,
  CreateIndex: ListTree,
  UpdateConfig: Settings2,
  Project: TableProperties,
  Restore: RotateCcw,
  Clone: Copy,
};

// Node colors per family: ring/background for the node, text for the icon and filter dot.
const FAMILY_STYLE: Record<OperationFamily, { node: string; dot: string }> = {
  write: {
    node: "border-emerald-500/40 bg-emerald-50 text-emerald-600 dark:bg-emerald-950 dark:text-emerald-400",
    dot: "bg-emerald-500",
  },
  remove: { node: "border-rose-500/40 bg-rose-50 text-rose-600 dark:bg-rose-950 dark:text-rose-400", dot: "bg-rose-500" },
  update: {
    node: "border-amber-500/40 bg-amber-50 text-amber-600 dark:bg-amber-950 dark:text-amber-400",
    dot: "bg-amber-500",
  },
  maintenance: { node: "border-sky-500/40 bg-sky-50 text-sky-600 dark:bg-sky-950 dark:text-sky-400", dot: "bg-sky-500" },
  metadata: {
    node: "border-violet-500/40 bg-violet-50 text-violet-600 dark:bg-violet-950 dark:text-violet-400",
    dot: "bg-violet-500",
  },
  restore: {
    node: "border-orange-500/40 bg-orange-50 text-orange-600 dark:bg-orange-950 dark:text-orange-400",
    dot: "bg-orange-500",
  },
  other: { node: "border-border bg-muted text-muted-foreground", dot: "bg-muted-foreground" },
};

// Width of the left rail; the node is centered on it and the connecting line runs through it.
const RAIL = "w-7";

/** Green for growth, red for shrinkage; `inverted` flips that for counts where growth is a
 *  removal (deleted rows). */
function DeltaChip({
  label,
  value,
  format,
  inverted = false,
}: {
  label: string;
  value: number | null;
  format: (n: number) => string;
  inverted?: boolean;
}) {
  if (value == null || value === 0) return null;
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1 rounded-md border px-1.5 py-0.5 text-xs tabular-nums",
        value > 0 !== inverted
          ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-700 dark:text-emerald-400"
          : "border-rose-500/30 bg-rose-500/10 text-rose-700 dark:text-rose-400",
      )}
    >
      {value > 0 ? "+" : "−"}
      {format(Math.abs(value))}
      <span className="opacity-70">{label}</span>
    </span>
  );
}

function Deltas({ delta }: { delta: SummaryDelta | null }) {
  if (!delta) {
    return (
      <span className="text-xs text-muted-foreground" title="Load earlier versions to compare">
        —
      </span>
    );
  }
  const chips = [
    <DeltaChip key="r" label="rows" value={delta.rows} format={formatCount} />,
    <DeltaChip key="b" label="data" value={delta.bytes} format={formatBytes} />,
    <DeltaChip key="f" label="fragments" value={delta.fragments} format={formatCount} />,
    <DeltaChip key="d" label="deleted rows" value={delta.deletedRows} format={formatCount} inverted />,
  ];
  if (!Object.values(delta).some((v) => v != null && v !== 0)) {
    return <span className="text-xs text-muted-foreground">No data change</span>;
  }
  return <div className="flex flex-wrap gap-1.5">{chips}</div>;
}

/** One `label: value` fact in the details list. */
function Fact({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-1 text-sm">
      <span className="shrink-0 text-xs text-muted-foreground">{label}</span>
      <span className="min-w-0">{children}</span>
    </div>
  );
}

function Mono({ children }: { children: React.ReactNode }) {
  return <code className="rounded bg-muted px-1 py-0.5 font-mono text-xs wrap-break-word">{children}</code>;
}

function NameList({ names }: { names: string[] }) {
  return (
    <span className="inline-flex flex-wrap gap-1">
      {names.map((n) => (
        <Mono key={n}>{n}</Mono>
      ))}
    </span>
  );
}

function CountFact({ label, value }: { label: string; value: unknown }) {
  const n = typeof value === "number" ? value : null;
  if (n == null) return null;
  return (
    <Fact label={label}>
      <span className="tabular-nums">{formatCount(n)}</span>
    </Fact>
  );
}

/** Keys set and removed in one metadata map. `set` is a map, or a key list for schema metadata. */
function MapChange({ label, change }: { label: string; change: Record<string, unknown> | null }) {
  if (!change) return null;
  const setMap = asRecord(change.set);
  const setKeys = setMap ? Object.keys(setMap) : asStrings(change.set);
  const removed = asStrings(change.removed);
  if (setKeys.length === 0 && removed.length === 0) return null;
  return (
    <Fact label={label}>
      <span className="flex flex-col gap-1">
        {setKeys.map((k) => (
          <span key={`s-${k}`} className="flex min-w-0 flex-wrap items-baseline gap-1">
            <span className="text-xs text-emerald-600 dark:text-emerald-400">set</span>
            <Mono>{k}</Mono>
            {setMap && (
              <>
                <span className="text-muted-foreground">=</span>
                <Mono>{asString(setMap[k]) ?? JSON.stringify(setMap[k])}</Mono>
              </>
            )}
          </span>
        ))}
        {removed.map((k) => (
          <span key={`r-${k}`} className="flex items-baseline gap-1">
            <span className="text-xs text-rose-600 dark:text-rose-400">removed</span>
            <Mono>{k}</Mono>
          </span>
        ))}
      </span>
    </Fact>
  );
}

function Details({ op, d }: { op: string; d: Record<string, unknown> }) {
  const facts: React.ReactNode[] = [];
  switch (op) {
    case "Append":
      facts.push(
        <CountFact key="fa" label="Fragments added" value={d.fragments_added} />,
        <CountFact key="ra" label="Rows added" value={d.rows_added} />,
        <CountFact key="fi" label="Files added" value={d.files_added} />,
      );
      break;
    case "Delete":
      if (asString(d.predicate)) {
        facts.push(
          <Fact key="p" label="Predicate">
            <Mono>{asString(d.predicate)}</Mono>
          </Fact>,
        );
      }
      facts.push(
        <CountFact key="fu" label="Fragments updated" value={d.fragments_updated} />,
        <CountFact key="fr" label="Fragments removed" value={d.fragments_removed} />,
      );
      break;
    case "Update": {
      facts.push(
        <CountFact key="fu" label="Fragments updated" value={d.fragments_updated} />,
        <CountFact key="fr" label="Fragments removed" value={d.fragments_removed} />,
        <CountFact key="fa" label="Fragments added" value={d.fragments_added} />,
      );
      const fields = asStrings(d.fields_modified);
      if (fields.length) {
        facts.push(
          <Fact key="fm" label="Columns modified">
            <NameList names={fields} />
          </Fact>,
        );
      }
      break;
    }
    case "Overwrite":
      facts.push(
        <CountFact key="f" label="Fragments" value={d.fragments} />,
        <CountFact key="r" label="Rows" value={d.rows} />,
        <CountFact key="c" label="Columns" value={d.fields} />,
      );
      break;
    case "CreateIndex": {
      const added = Array.isArray(d.added) ? d.added.map(asRecord).filter((x) => x != null) : [];
      const removed = asStrings(d.removed);
      for (const [i, ix] of added.entries()) {
        const cols = asStrings(ix.columns);
        facts.push(
          <Fact key={`a${i}`} label="Index">
            <span className="inline-flex flex-wrap items-baseline gap-1.5">
              <Mono>{asString(ix.name) ?? "unnamed"}</Mono>
              {asString(ix.type) && <Badge variant="outline">{asString(ix.type)}</Badge>}
              {cols.length > 0 && (
                <>
                  <span className="text-xs text-muted-foreground">on</span>
                  <NameList names={cols} />
                </>
              )}
            </span>
          </Fact>,
        );
      }
      if (removed.length) {
        facts.push(
          <Fact key="rm" label="Replaced">
            <NameList names={removed} />
          </Fact>,
        );
      }
      break;
    }
    case "Rewrite":
      facts.push(
        <CountFact key="g" label="Groups" value={d.groups} />,
        <CountFact key="b" label="Fragments before" value={d.fragments_before} />,
        <CountFact key="a" label="Fragments after" value={d.fragments_after} />,
        <CountFact key="i" label="Indices rewritten" value={d.rewritten_indices} />,
      );
      break;
    case "Merge":
      facts.push(
        <CountFact key="f" label="Fragments" value={d.fragments} />,
        <CountFact key="c" label="Columns" value={d.fields} />,
      );
      break;
    case "Project": {
      const fields = asStrings(d.fields);
      if (fields.length) {
        facts.push(
          <Fact key="f" label="Columns">
            <NameList names={fields} />
          </Fact>,
        );
      }
      break;
    }
    case "UpdateConfig": {
      facts.push(
        <MapChange key="c" label="Config" change={asRecord(d.config)} />,
        <MapChange key="t" label="Table metadata" change={asRecord(d.table_metadata)} />,
        <MapChange key="s" label="Schema metadata" change={asRecord(d.schema_metadata)} />,
      );
      const fields = asStrings(asRecord(d.field_metadata)?.fields);
      if (fields.length) {
        facts.push(
          <Fact key="fm" label="Field metadata">
            <NameList names={fields} />
          </Fact>,
        );
      }
      break;
    }
    case "Restore":
      if (typeof d.version === "number") {
        facts.push(
          <Fact key="v" label="Restored to">
            <Badge variant="outline">v{d.version}</Badge>
          </Fact>,
        );
      }
      break;
    case "DataReplacement":
      facts.push(<CountFact key="r" label="Replacements" value={d.replacements} />);
      break;
  }
  if (facts.length === 0) return <span className="text-sm text-muted-foreground">None recorded</span>;
  return <div className="space-y-1.5">{facts}</div>;
}

/** One labeled section of the detail pane. */
function PaneSection({ title, children }: { title: string; children: React.ReactNode }) {
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

interface Entry {
  event: TimelineEvent;
  delta: SummaryDelta | null;
}

/** Everything recorded for one version. Sticky beside the list on wide screens, an overlay
 *  on narrow ones. */
function DetailPane({ entry, zone, onClose }: { entry: Entry; zone: TimeZonePref; onClose: () => void }) {
  const { event, delta } = entry;
  const ref = useRef<HTMLElement>(null);
  const [maxHeight, setMaxHeight] = useState<number>();
  // Fit the sticky pane to the page's scroll area, which shrinks as the SQL dock grows.
  useEffect(() => {
    const main = ref.current?.closest("main");
    if (!main) return;
    const observer = new ResizeObserver(() => setMaxHeight(main.clientHeight - 32));
    observer.observe(main);
    return () => observer.disconnect();
  }, []);
  const Icon = ICON[event.operation] ?? CircleDot;
  const style = FAMILY_STYLE[operationFamily(event.operation)];
  const date = event.timestamp_millis == null ? null : new Date(event.timestamp_millis);
  const props = Object.entries(event.transaction?.properties ?? {});

  return (
    <>
      <div aria-hidden className="fixed inset-0 z-40 bg-black/30 xl:hidden" onClick={onClose} />
      <aside
        ref={ref}
        aria-label={`Version ${event.version} details`}
        style={{ "--pane-max": maxHeight ? `${maxHeight}px` : "100%" } as React.CSSProperties}
        className="fixed inset-y-0 right-0 z-50 flex w-full max-w-[420px] flex-col overflow-hidden border-l bg-background shadow-xl xl:sticky xl:top-4 xl:z-10 xl:max-h-(--pane-max) xl:w-[420px] xl:shrink-0 xl:self-start xl:rounded-lg xl:border xl:shadow-sm"
      >
        <div className="flex items-start gap-3 px-4 py-3">
          <span className={cn("flex h-8 w-8 shrink-0 items-center justify-center rounded-full border", style.node)}>
            <Icon className="h-4 w-4" />
          </span>
          <div className="min-w-0 flex-1">
            <div className="flex flex-wrap items-center gap-2">
              <span className="font-medium">{operationLabel(event.operation)}</span>
              <Badge variant="outline" className="font-mono">
                v{event.version}
              </Badge>
            </div>
            <div className="font-mono text-xs text-muted-foreground">{event.operation}</div>
          </div>
          <Button variant="ghost" size="icon" className="h-7 w-7" onClick={onClose} aria-label="Close details">
            <X />
          </Button>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto">
          <PaneSection title="Commit">
            <Fact label="Time">
              <span className="font-mono text-xs tabular-nums">{date ? formatIsoTimestamp(date, zone) : "Unknown"}</span>
            </Fact>
            <Fact label="Version">
              <span className="font-mono text-xs">v{event.version}</span>
            </Fact>
            {event.transaction && (
              <Fact label="Read version">
                <span className="font-mono text-xs">v{event.transaction.read_version}</span>
              </Fact>
            )}
            {event.tags.length > 0 && (
              <Fact label="Tags">
                <span className="inline-flex flex-wrap gap-1">
                  {event.tags.map((t) => (
                    <Badge key={t} className="font-mono">
                      {t}
                    </Badge>
                  ))}
                </span>
              </Fact>
            )}
          </PaneSection>
          <PaneSection title="Change from previous loaded version">
            <Deltas delta={delta} />
          </PaneSection>
          <PaneSection title="Operation details">
            <Details op={event.operation} d={event.details} />
          </PaneSection>
          <PaneSection title="Totals at this version">
            <dl className="grid grid-cols-2 gap-x-4 gap-y-1.5">
              {SUMMARY_FIELDS.map(([key, label, bytes]) => {
                const raw = event.summary[key];
                const n = raw == null || raw === "" ? null : Number(raw);
                return (
                  <div key={key} className="min-w-0">
                    <dt className="text-xs text-muted-foreground">{label}</dt>
                    <dd className="text-sm tabular-nums">{bytes ? formatBytes(n) : formatCount(n)}</dd>
                  </div>
                );
              })}
            </dl>
          </PaneSection>
          <PaneSection title="Writer and transaction">
            <Fact label="Writer">
              {event.writer ? `${event.writer.library} ${event.writer.version}` : <span className="text-muted-foreground">Unknown</span>}
            </Fact>
            {event.transaction ? (
              <>
                <Fact label="Transaction">
                  <span className="inline-flex items-center gap-1">
                    <Mono>{event.transaction.uuid}</Mono>
                    <CopyButton value={event.transaction.uuid} label="Copy transaction id" />
                  </span>
                </Fact>
                {props.length > 0 && (
                  <Fact label="Properties">
                    <span className="flex flex-col gap-1">
                      {props.map(([k, v]) => (
                        <span key={k} className="flex min-w-0 flex-wrap items-baseline gap-1">
                          <Mono>{k}</Mono>
                          <span className="text-muted-foreground">=</span>
                          <Mono>{v}</Mono>
                        </span>
                      ))}
                    </span>
                  </Fact>
                )}
              </>
            ) : (
              <Fact label="Transaction">
                <span className="text-muted-foreground">Not recorded</span>
              </Fact>
            )}
          </PaneSection>
        </div>
      </aside>
    </>
  );
}

function EventRow({
  entry,
  zone,
  selected,
  onSelect,
  rowRef,
}: {
  entry: Entry;
  zone: TimeZonePref;
  selected: boolean;
  onSelect: () => void;
  rowRef: (el: HTMLLIElement | null) => void;
}) {
  const { event, delta } = entry;
  const Icon = ICON[event.operation] ?? CircleDot;
  const style = FAMILY_STYLE[operationFamily(event.operation)];
  const summary = eventSummary(event, delta);
  return (
    <li
      ref={rowRef}
      tabIndex={0}
      aria-current={selected || undefined}
      onClick={onSelect}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onSelect();
        }
      }}
      className={cn(
        "group relative flex cursor-pointer items-center gap-3 rounded-md py-1 pr-2 outline-none focus-visible:ring-2 focus-visible:ring-ring",
        selected ? "bg-accent" : "hover:bg-accent/50",
      )}
    >
      <div className={cn("flex shrink-0 justify-center", RAIL)}>
        <span
          className={cn("z-10 flex h-6 w-6 items-center justify-center rounded-full border ring-2 ring-background", style.node)}
        >
          <Icon className="h-3.5 w-3.5" />
        </span>
      </div>
      <div className="flex min-w-0 flex-1 items-center gap-2">
        <span className="shrink-0 text-sm font-medium">{operationLabel(event.operation)}</span>
        <Badge variant="outline" className="shrink-0 font-mono">
          v{event.version}
        </Badge>
        {event.tags.map((t) => (
          <Badge key={t} className="shrink-0 font-mono">
            {t}
          </Badge>
        ))}
        {summary && <span className="min-w-0 truncate text-xs text-muted-foreground">{summary}</span>}
      </div>
      <time
        dateTime={event.timestamp_millis == null ? undefined : new Date(event.timestamp_millis).toISOString()}
        className="shrink-0 font-mono text-xs text-muted-foreground tabular-nums"
      >
        {event.timestamp_millis == null ? "—" : formatIsoTime(new Date(event.timestamp_millis), zone)}
      </time>
    </li>
  );
}

/** Sticky day divider centered on the rail: the ISO date, the day's event count, and a count per
 *  operation family in the same colors as the row markers. */
function DayHeader({ group }: { group: DayGroup }) {
  const families = new Map<OperationFamily, number>();
  let events = 0;
  for (const r of group.rows) {
    if (r.kind !== "event") continue;
    events++;
    const family = operationFamily(r.event.operation);
    families.set(family, (families.get(family) ?? 0) + 1);
  }
  return (
    <h3 className="sticky top-0 z-20 flex items-center gap-3 rounded-md border bg-card/95 py-1.5 pr-3 shadow-sm backdrop-blur supports-[backdrop-filter]:bg-card/80">
      <div className={cn("flex shrink-0 justify-center", RAIL)}>
        <span className="z-10 flex h-6 w-6 items-center justify-center rounded-md border bg-muted text-muted-foreground ring-2 ring-card">
          <CalendarDays className="h-3.5 w-3.5" />
        </span>
      </div>
      {group.day === "unknown" ? (
        <span className="text-sm font-semibold text-foreground">Unknown date</span>
      ) : (
        <time dateTime={group.day} className="font-mono text-sm font-semibold tracking-tight text-foreground">
          {group.day}
        </time>
      )}
      <span className="rounded-full bg-muted px-2 py-0.5 text-[11px] font-medium text-muted-foreground tabular-nums">
        {formatCountOf(events, "event")}
      </span>
      <span className="ml-auto flex items-center gap-3">
        {[...families].map(([family, n]) => (
          <span key={family} className="flex items-center gap-1 text-[11px] text-muted-foreground tabular-nums">
            <span className={cn("h-1.5 w-1.5 rounded-full", FAMILY_STYLE[family].dot)} />
            {formatCount(n)}
          </span>
        ))}
      </span>
    </h3>
  );
}

function GapRow({ from, to }: { from: number; to: number }) {
  return (
    <li className="relative flex items-center gap-3 py-1">
      <div className={cn("flex shrink-0 justify-center", RAIL)}>
        <span className="z-10 flex h-5 w-5 items-center justify-center rounded-full border border-dashed bg-background text-muted-foreground">
          <Scissors className="h-3 w-3" />
        </span>
      </div>
      <div className="flex-1 rounded-md border border-dashed px-2.5 py-1 text-xs text-muted-foreground">
        {from === to ? `Version v${from}` : `Versions v${from}–v${to}`} not retained (removed by cleanup)
      </div>
    </li>
  );
}

function TimelineSkeleton() {
  return (
    <ol className="space-y-2" aria-label="Loading timeline">
      {Array.from({ length: 10 }, (_, i) => (
        <li key={i} className="flex items-center gap-3">
          <div className={cn("flex shrink-0 justify-center", RAIL)}>
            <Skeleton className="h-6 w-6 rounded-full" />
          </div>
          <Skeleton className="h-4 w-28" />
          <Skeleton className="h-4 w-10" />
          <Skeleton className="h-3 w-40" />
          <Skeleton className="ml-auto h-3 w-16" />
        </li>
      ))}
    </ol>
  );
}

/** Every retained version as a compact commit row, newest first, grouped by day, with cleanup
 *  gaps. Selecting a row opens its details beside the list. Pages load through "Load earlier". */
export function TimelineTab({ segments }: { segments: readonly string[] }) {
  const timeline = useTableTimeline(segments);
  const zone = useTimeZonePref();
  const [filter, setFilter] = useState<ReadonlySet<string>>(new Set());
  const [active, setActive] = useState<number | null>(null);
  const rowRefs = useRef(new Map<number, HTMLLIElement>());
  const events = useMemo(() => timeline.data?.pages.flatMap((p) => p.events) ?? [], [timeline.data]);
  const counts = useMemo(() => operationCounts(events), [events]);
  const rows = useMemo(() => {
    const keep = filter.size ? (e: TimelineEvent) => filter.has(e.operation) : undefined;
    return buildTimelineRows(events, keep);
  }, [events, filter]);
  const groups = useMemo(() => groupByDay(rows, zone), [rows, zone]);
  const visible = useMemo(() => rows.filter((r) => r.kind === "event"), [rows]);
  const index = visible.findIndex((r) => r.event.version === active);
  const current = index >= 0 ? visible[index] : null;

  // While the pane is open: Esc closes it, arrows move the selection. Typing elsewhere (the
  // SQL editor, inputs) is left alone.
  useEffect(() => {
    if (!current) return;
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null;
      if (t && (t.isContentEditable || t.closest("input, textarea, select, [role=dialog]"))) return;
      if (e.key === "Escape") {
        e.preventDefault();
        setActive(null);
        rowRefs.current.get(current.event.version)?.focus();
        return;
      }
      const step = e.key === "ArrowDown" ? 1 : e.key === "ArrowUp" ? -1 : 0;
      const next = visible[index + step];
      if (!step || !next) return;
      e.preventDefault();
      setActive(next.event.version);
      const el = rowRefs.current.get(next.event.version);
      el?.focus({ preventScroll: true });
      el?.scrollIntoView({ block: "nearest" });
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [current, index, visible]);

  if (timeline.isPending) return <TimelineSkeleton />;
  if (timeline.error && events.length === 0) {
    return <ErrorState message={`Timeline unavailable: ${timeline.error.message}`} />;
  }
  if (events.length === 0) {
    return (
      <EmptyState icon={GitCommitVertical} title="No versions">
        The table reports no retained versions.
      </EmptyState>
    );
  }

  const toggle = (op: string) =>
    setFilter((cur) => {
      const next = new Set(cur);
      if (!next.delete(op)) next.add(op);
      return next;
    });
  const close = () => {
    if (current) rowRefs.current.get(current.event.version)?.focus();
    setActive(null);
  };

  return (
    <div className="flex items-start gap-4">
      <div className="min-w-0 flex-1 space-y-3">
        <div className="flex flex-wrap items-center gap-2">
          {counts.map(([op, n]) => {
            const on = filter.has(op);
            return (
              <button
                key={op}
                type="button"
                aria-pressed={on}
                onClick={() => toggle(op)}
                className={cn(
                  "inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-xs transition-colors",
                  on ? "border-foreground/30 bg-accent text-foreground" : "text-muted-foreground hover:bg-accent/60",
                )}
              >
                <span className={cn("h-2 w-2 rounded-full", FAMILY_STYLE[operationFamily(op)].dot)} />
                {operationLabel(op)}
                <span className="tabular-nums opacity-70">{n}</span>
              </button>
            );
          })}
          {filter.size > 0 && (
            <Button variant="ghost" size="sm" className="h-7 text-xs" onClick={() => setFilter(new Set())}>
              Clear
            </Button>
          )}
          <span className="ml-auto text-xs text-muted-foreground tabular-nums">
            {filter.size > 0 ? `${formatCount(visible.length)} of ` : ""}
            {formatCount(events.length)} loaded
            {timeline.hasNextPage ? "" : " · all versions"}
          </span>
        </div>

        {groups.length === 0 ? (
          <EmptyState title="No matching entries">No loaded versions match the selected operations.</EmptyState>
        ) : (
          <div className="relative">
            {/* The rail's connecting line, behind the nodes. */}
            <div aria-hidden className={cn("absolute top-3 bottom-3 left-0 flex justify-center", RAIL)}>
              <div className="w-px bg-border" />
            </div>
            {groups.map((g) => (
              <section key={g.day} className="relative">
                <DayHeader group={g} />
                <ol className="mb-2">
                  {g.rows.map((r) =>
                    r.kind === "event" ? (
                      <EventRow
                        key={`e${r.event.version}`}
                        entry={r}
                        zone={zone}
                        selected={r.event.version === current?.event.version}
                        onSelect={() => setActive(r.event.version)}
                        rowRef={(el) => {
                          if (el) rowRefs.current.set(r.event.version, el);
                          else rowRefs.current.delete(r.event.version);
                        }}
                      />
                    ) : (
                      <GapRow key={`g${r.from}`} from={r.from} to={r.to} />
                    ),
                  )}
                </ol>
              </section>
            ))}
          </div>
        )}

        {timeline.error && <ErrorState message={`Could not load earlier versions: ${timeline.error.message}`} />}
        {timeline.hasNextPage && (
          <div className="flex justify-center">
            <Button
              variant="outline"
              size="sm"
              disabled={timeline.isFetchingNextPage}
              onClick={() => void timeline.fetchNextPage()}
            >
              {timeline.isFetchingNextPage ? "Loading…" : "Load earlier"}
            </Button>
          </div>
        )}
      </div>
      {current && <DetailPane key={current.event.version} entry={current} zone={zone} onClose={close} />}
    </div>
  );
}
