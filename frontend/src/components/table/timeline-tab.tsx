import { useMemo, useState } from "react";
import { CalendarDays, CircleDot, GitCommitVertical, Scissors } from "lucide-react";
import { useTableTimeline } from "@/lib/catalog-queries";
import { formatCount, formatCountOf, formatIsoTime, type TimeZonePref } from "@/lib/format";
import { useTimeZonePref } from "@/lib/preferences";
import type { TimelineEvent } from "@/lib/schemas";
import {
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
import { SidePaneLayout, usePaneSelection, type PaneRowProps } from "@/components/side-pane";
import { FAMILY_STYLE, ICON, VersionDetailPane, type PaneView } from "./version-pane";

// Width of the left rail; the node is centered on it and the connecting line runs through it.
const RAIL = "w-7";

interface Entry {
  event: TimelineEvent;
  delta: SummaryDelta | null;
}

const eventVersion = (entry: Entry) => entry.event.version;

function EventRow({
  entry,
  zone,
  selected,
  rowProps,
}: {
  entry: Entry;
  zone: TimeZonePref;
  selected: boolean;
  rowProps: PaneRowProps;
}) {
  const { event, delta } = entry;
  const Icon = ICON[event.operation] ?? CircleDot;
  const style = FAMILY_STYLE[operationFamily(event.operation)];
  const summary = eventSummary(event, delta);
  return (
    <li
      {...rowProps}
      aria-current={selected || undefined}
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
    <h3 className="sticky top-(--tabs-h,0px) z-20 flex items-center gap-3 rounded-md border bg-card/95 py-1.5 pr-3 shadow-sm backdrop-blur supports-[backdrop-filter]:bg-card/80">
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
  const [view, setView] = useState<PaneView>("details");
  const events = useMemo(() => timeline.data?.pages.flatMap((p) => p.events) ?? [], [timeline.data]);
  const counts = useMemo(() => operationCounts(events), [events]);
  const rows = useMemo(() => {
    const keep = filter.size ? (e: TimelineEvent) => filter.has(e.operation) : undefined;
    return buildTimelineRows(events, keep);
  }, [events, filter]);
  const groups = useMemo(() => groupByDay(rows, zone), [rows, zone]);
  const visible = useMemo(() => rows.filter((r) => r.kind === "event"), [rows]);
  const { current, close, rowProps } = usePaneSelection(visible, eventVersion);

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

  return (
    <SidePaneLayout
      className="space-y-3"
      pane={
        current && (
          <VersionDetailPane
            segments={segments}
            version={current.event.version}
            event={current.event}
            delta={current.delta}
            view={view}
            onView={setView}
            onClose={close}
          />
        )
      }
    >
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
                      rowProps={rowProps(r.event.version)}
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
    </SidePaneLayout>
  );
}
