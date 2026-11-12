import { useEffect, useRef, useState } from "react";
import type { UseQueryResult } from "@tanstack/react-query";
import { BarChart3, Layers } from "lucide-react";
import { useTableBranches, useTableTags, type PagedVersions } from "@/lib/catalog-queries";
import { formatBytes, formatCount, formatRelative, parseTime } from "@/lib/format";
import { useDateTimeFormatter } from "@/lib/preferences";
import { parseManifestSummary, versionSeries, versionStatus } from "@/lib/table-metadata";
import { summaryDelta } from "@/lib/timeline";
import type { TableVersion } from "@/lib/schemas";
import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { EmptyState } from "@/components/empty-state";
import { ErrorState } from "@/components/error-state";
import { Section } from "@/components/section";
import { GrowthChart } from "./charts";
import { STICKY_HEAD as HEAD } from "./sticky-head";
import { VersionDetailPane, type PaneView } from "./version-pane";

function Delta({ value }: { value: number | null }) {
  if (value == null || value === 0) return <span className="text-muted-foreground/60">—</span>;
  return (
    <span className={cn(value > 0 ? "text-emerald-600 dark:text-emerald-400" : "text-rose-600 dark:text-rose-400")}>
      {value > 0 ? "+" : "−"}
      {formatCount(Math.abs(value))}
    </span>
  );
}

/** Rows and data size across the loaded versions. */
function TrendsSection({ loaded }: { loaded: readonly TableVersion[] }) {
  const series = versionSeries(loaded);
  return (
    <Section
      title="Trends"
      help="Rows and data size of the loaded versions."
      actions={
        <div className="flex flex-col items-end gap-0.5 text-xs text-muted-foreground">
          <span className="flex items-center gap-1.5">
            <span className="h-0.5 w-4 rounded bg-[hsl(var(--chart-1))]" /> Rows (left axis)
          </span>
          <span className="flex items-center gap-1.5">
            <span className="h-0 w-4 border-t-2 border-dashed border-[hsl(var(--chart-2))]" /> Data size (right axis)
          </span>
        </div>
      }
      className="shadow-sm"
      contentClassName="pb-3"
    >
      {series.every((p) => p.rows == null && p.bytes == null) ? (
        <EmptyState icon={BarChart3} title="No version history">No loaded version reports rows or size.</EmptyState>
      ) : (
        <GrowthChart points={series} />
      )}
    </Section>
  );
}

/** Named list of refs: one `name  detail` line each, or an empty note. */
function RefsSection<T>({
  title,
  help,
  query,
  entries,
  empty,
}: {
  title: string;
  help: string;
  query: UseQueryResult<T>;
  entries: (data: T) => [string, string][];
  empty: string;
}) {
  return (
    <Section title={title} help={help}>
      {query.error ? (
        <ErrorState message={query.error.message} />
      ) : query.isPending ? (
        <Skeleton className="h-4 w-32" />
      ) : entries(query.data).length === 0 ? (
        <p className="text-sm text-muted-foreground/70 italic">{empty}</p>
      ) : (
        <ul className="space-y-1">
          {entries(query.data).map(([name, detail]) => (
            <li key={name} className="flex items-baseline justify-between gap-3 font-mono text-xs">
              <span className="truncate">{name}</span>
              <span className="shrink-0 text-muted-foreground tabular-nums">{detail}</span>
            </li>
          ))}
        </ul>
      )}
    </Section>
  );
}

function RefsTiles({ segments }: { segments: readonly string[] }) {
  const tags = useTableTags(segments);
  const branches = useTableBranches(segments);
  const byName = <V,>(m: Record<string, V>) => Object.entries(m).sort(([a], [b]) => a.localeCompare(b));
  return (
    <div className="mb-6 grid gap-6 @3xl:grid-cols-2">
      <RefsSection
        title="Tags"
        help="Named versions."
        query={tags}
        empty="No tags"
        entries={(t) => byName(t.tags).map(([name, tag]) => [name, `${tag.branch ? `${tag.branch}@` : ""}v${tag.version}`])}
      />
      <RefsSection
        title="Branches"
        help="Branches and the version each started from."
        query={branches}
        empty="No branches"
        entries={(b) =>
          byName(b.branches).map(([name, br]) => [
            name,
            `from ${br.parentBranch ?? "main"}${br.parentVersion != null ? ` v${br.parentVersion}` : ""}${br.manifestSize != null ? ` · ${formatBytes(br.manifestSize)}` : ""}`,
          ])
        }
      />
    </div>
  );
}

/** Loaded versions, newest first, with "Load earlier" for the next page. Selecting a row opens
 *  its pane. */
function VersionHistory({
  versions,
  currentVersion,
  active,
  onSelect,
  rowRefs,
}: {
  versions: PagedVersions;
  currentVersion: number | null | undefined;
  active: number | null;
  onSelect: (version: number) => void;
  rowRefs: Map<number, HTMLTableRowElement>;
}) {
  const formatDateTime = useDateTimeFormatter();
  const all = versions.versions;
  const full = active == null;
  return (
    <Section
      title="Version history"
      description={versions.hasNextPage ? `${formatCount(all.length)} loaded` : undefined}
      help="The Current badge marks this version; newer versions need a catalog refresh."
      contentClassName="px-0 pb-0"
    >
      {/* A plain table (no scroll wrapper) so its header can stick below the Trends chart. While a
          pane is open beside it, only the leading columns fit. */}
      <table className="w-full text-sm">
        <thead>
          <tr>
            <th className={cn(HEAD, "pl-4")}>Version</th>
            <th className={HEAD}>Created</th>
            <th className={cn(HEAD, "text-right")}>Rows</th>
            {full && <th className={cn(HEAD, "text-right")}>Δ Rows</th>}
            <th className={cn(HEAD, "text-right", !full && "pr-4")}>Data size</th>
            {full && <th className={cn(HEAD, "text-right")}>Data files</th>}
            {full && <th className={cn(HEAD, "pr-4 text-right")}>Deletions</th>}
          </tr>
        </thead>
        <tbody className="tabular-nums [&_td]:p-2 [&_td]:align-middle [&_td]:whitespace-nowrap [&_tr]:border-b [&_tr:last-child]:border-0">
          {all.map((v, i) => {
            const s = parseManifestSummary(v.metadata);
            // Newest first, so the previous version is the next loaded entry.
            const prev = all[i + 1];
            const prevRows = prev ? parseManifestSummary(prev.metadata).rows : null;
            const delta = s.rows != null && prevRows != null ? s.rows - prevRows : null;
            const date = parseTime(v.timestamp_millis);
            const status = versionStatus(v.version, currentVersion);
            return (
              <tr
                key={v.version}
                ref={(el) => {
                  if (el) rowRefs.set(v.version, el);
                  else rowRefs.delete(v.version);
                }}
                tabIndex={0}
                aria-selected={v.version === active}
                onClick={() => onSelect(v.version)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    onSelect(v.version);
                  }
                }}
                className={cn(
                  "cursor-pointer transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset",
                  v.version === active ? "bg-accent" : "hover:bg-muted/50",
                )}
              >
                <td className="pl-4">
                  <div className="flex items-center gap-2">
                    <span className="font-mono font-medium">v{v.version}</span>
                    {status === "current" && <Badge variant="secondary">current</Badge>}
                    {status === "newer" && <Badge variant="outline">newer than current</Badge>}
                  </div>
                </td>
                <td>
                  <div>{formatRelative(date)}</div>
                  <div className="text-xs text-muted-foreground">{formatDateTime(date)}</div>
                </td>
                <td className="text-right">{formatCount(s.rows)}</td>
                {full && (
                  <td className="text-right">
                    <Delta value={delta} />
                  </td>
                )}
                <td className={cn("text-right", !full && "pr-4")}>{formatBytes(s.dataBytes)}</td>
                {full && <td className="text-right">{formatCount(s.dataFiles)}</td>}
                {full && (
                  <td className="pr-4 text-right">
                    {s.deletionFiles ? (
                      <>
                        {formatCount(s.deletedRows)} rows
                        <div className="text-xs text-muted-foreground">{formatCount(s.deletionFiles)} files</div>
                      </>
                    ) : (
                      <span className="text-muted-foreground/60">{s.deletionFiles === 0 ? "none" : "—"}</span>
                    )}
                  </td>
                )}
              </tr>
            );
          })}
        </tbody>
      </table>
      {(versions.hasNextPage || versions.error) && (
        <div className="space-y-2 border-t px-4 py-3">
          {versions.error && <ErrorState message={`Could not load earlier versions: ${versions.error.message}`} />}
          {versions.hasNextPage && (
            <div className="flex justify-center">
              <Button variant="outline" size="sm" disabled={versions.isFetchingNextPage} onClick={versions.fetchNextPage}>
                {versions.isFetchingNextPage ? "Loading…" : "Load earlier"}
              </Button>
            </div>
          )}
        </div>
      )}
    </Section>
  );
}

/** Trends over the loaded versions (sticky while the table scrolls), then tags and branches, then
 *  the version table with the selected version's pane beside it. */
export function VersionsTab({
  segments,
  versions,
  currentVersion,
}: {
  segments: readonly string[];
  versions: PagedVersions;
  currentVersion: number | null | undefined;
}) {
  const trends = useRef<HTMLDivElement>(null);
  const [trendsHeight, setTrendsHeight] = useState(0);
  const [active, setActive] = useState<number | null>(null);
  const [view, setView] = useState<PaneView>("details");
  const rowRefs = useRef(new Map<number, HTMLTableRowElement>());
  const all = versions.versions;
  const index = all.findIndex((v) => v.version === active);
  const current = index >= 0 ? all[index] : null;
  const ready = !versions.isPending && all.length > 0;
  // The table header sticks just below the sticky chart, whose height follows the viewport.
  useEffect(() => {
    const el = trends.current;
    if (!el) return;
    const observer = new ResizeObserver(() => setTrendsHeight(el.offsetHeight));
    observer.observe(el);
    return () => observer.disconnect();
  }, [ready]);

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
        rowRefs.current.get(current.version)?.focus();
        return;
      }
      const step = e.key === "ArrowDown" ? 1 : e.key === "ArrowUp" ? -1 : 0;
      const next = all[index + step];
      if (!step || !next) return;
      e.preventDefault();
      setActive(next.version);
      const el = rowRefs.current.get(next.version);
      el?.focus({ preventScroll: true });
      el?.scrollIntoView({ block: "nearest" });
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [current, index, all]);

  if (versions.isPending) {
    return (
      <div className="space-y-6">
        <Skeleton className="h-56 w-full" />
        <div className="space-y-2">
          {Array.from({ length: 8 }, (_, i) => (
            <Skeleton key={i} className="h-9 w-full" />
          ))}
        </div>
      </div>
    );
  }
  if (all.length === 0) {
    if (versions.error) return <ErrorState message={`Versions unavailable: ${versions.error.message}`} />;
    return <EmptyState icon={Layers} title="No versions">The table reports no versions.</EmptyState>;
  }
  const close = () => {
    if (current) rowRefs.current.get(current.version)?.focus();
    setActive(null);
  };
  return (
    // The table header pins just below the pinned Trends chart.
    <div className="@container" style={{ "--sticky-top": `${trendsHeight}px` } as React.CSSProperties}>
      <div ref={trends} className="sticky top-(--tabs-h,0px) z-20 -mx-1 bg-background px-1 pt-1 pb-4">
        <TrendsSection loaded={all} />
      </div>
      <RefsTiles segments={segments} />
      <div className="flex items-start gap-4">
        <div className="min-w-0 flex-1">
          <VersionHistory
            versions={versions}
            currentVersion={currentVersion}
            active={current?.version ?? null}
            onSelect={setActive}
            rowRefs={rowRefs.current}
          />
        </div>
        {current && (
          <VersionDetailPane
            key={current.version}
            segments={segments}
            version={current.version}
            delta={
              all[index + 1]
                ? summaryDelta({ summary: current.metadata ?? {} }, { summary: all[index + 1].metadata ?? {} })
                : null
            }
            view={view}
            onView={setView}
            onClose={close}
            top={trendsHeight}
          />
        )}
      </div>
    </div>
  );
}
