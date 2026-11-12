import { memo, useEffect, useMemo, useRef, useState } from "react";
import type { UseQueryResult } from "@tanstack/react-query";
import { BarChart3, Layers } from "lucide-react";
import { useTableBranches, useTableTags, type PagedVersions } from "@/lib/catalog-queries";
import { formatBytes, formatCount, formatRelative, parseTime } from "@/lib/format";
import { useDateTimeFormatter } from "@/lib/preferences";
import { parseManifestSummary, sortedEntries, versionSeries, versionStatus } from "@/lib/table-metadata";
import { summaryDelta } from "@/lib/timeline";
import type { TableVersion } from "@/lib/schemas";
import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Dash, EmptyState } from "@/components/empty-state";
import { ErrorState } from "@/components/error-state";
import { QueryBody } from "@/components/query-body";
import { Section } from "@/components/section";
import { SidePaneLayout, usePaneSelection, type PaneRowProps } from "@/components/side-pane";
import { GrowthChart } from "./charts";
import { HEAD_CELL, ScrollTable } from "./scroll-table";
import { VersionDetailPane, type PaneView } from "./version-pane";

const versionOf = (v: TableVersion) => v.version;

function Delta({ value }: { value: number | null }) {
  if (value == null || value === 0) return <Dash />;
  return (
    <span className={cn(value > 0 ? "text-emerald-600 dark:text-emerald-400" : "text-rose-600 dark:text-rose-400")}>
      {value > 0 ? "+" : "−"}
      {formatCount(Math.abs(value))}
    </span>
  );
}

const TrendsSection = memo(function TrendsSection({ loaded }: { loaded: readonly TableVersion[] }) {
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
});

function RefList({ entries, empty }: { entries: [string, string][]; empty: string }) {
  if (entries.length === 0) return <p className="text-sm text-muted-foreground/70 italic">{empty}</p>;
  return (
    <ul className="space-y-1">
      {entries.map(([name, detail]) => (
        <li key={name} className="flex items-baseline justify-between gap-3 font-mono text-xs">
          <span className="truncate">{name}</span>
          <span className="shrink-0 text-muted-foreground tabular-nums">{detail}</span>
        </li>
      ))}
    </ul>
  );
}

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
      <QueryBody query={query} skeletonClassName="h-4 w-32">
        {(data) => <RefList entries={entries(data)} empty={empty} />}
      </QueryBody>
    </Section>
  );
}

const RefsTiles = memo(function RefsTiles({ segments }: { segments: readonly string[] }) {
  const tags = useTableTags(segments);
  const branches = useTableBranches(segments);
  return (
    <div className="mb-6 grid gap-6 @3xl:grid-cols-2">
      <RefsSection
        title="Tags"
        help="Named versions."
        query={tags}
        empty="No tags"
        entries={(t) =>
          sortedEntries(t.tags).map(([name, tag]) => [name, `${tag.branch ? `${tag.branch}@` : ""}v${tag.version}`])
        }
      />
      <RefsSection
        title="Branches"
        help="Branches and the version each started from."
        query={branches}
        empty="No branches"
        entries={(b) =>
          sortedEntries(b.branches).map(([name, br]) => [
            name,
            `from ${br.parentBranch ?? "main"}${br.parentVersion != null ? ` v${br.parentVersion}` : ""}${br.manifestSize != null ? ` · ${formatBytes(br.manifestSize)}` : ""}`,
          ])
        }
      />
    </div>
  );
});

function VersionHistory({
  versions,
  currentVersion,
  active,
  rowProps,
}: {
  versions: PagedVersions;
  currentVersion: number | null | undefined;
  active: number | null;
  rowProps: (version: number) => PaneRowProps;
}) {
  const formatDateTime = useDateTimeFormatter();
  const all = versions.versions;
  const summaries = useMemo(() => all.map((v) => parseManifestSummary(v.metadata)), [all]);
  return (
    <Section
      title="Version history"
      description={versions.hasNextPage ? `${formatCount(all.length)} loaded` : undefined}
      help="The Current badge marks this version; Refresh on the Overview makes the newest version current."
      contentClassName="px-0 pb-0"
    >
      <ScrollTable
        className="w-full text-sm"
        head={
          <tr>
            <th className={cn(HEAD_CELL, "pl-4")}>Version</th>
            <th className={HEAD_CELL}>Created</th>
            <th className={cn(HEAD_CELL, "text-right")}>Rows</th>
            <th className={cn(HEAD_CELL, "text-right")}>Δ Rows</th>
            <th className={cn(HEAD_CELL, "text-right")}>Data size</th>
            <th className={cn(HEAD_CELL, "text-right")}>Data files</th>
            <th className={cn(HEAD_CELL, "pr-4 text-right")}>Deletions</th>
          </tr>
        }
      >
        <tbody className="tabular-nums [&_td]:p-2 [&_td]:align-middle [&_td]:whitespace-nowrap [&_tr]:border-b [&_tr:last-child]:border-0">
          {all.map((v, i) => {
            const s = summaries[i];
            // Newest first, so the previous version is the next loaded entry.
            const prevRows = summaries[i + 1]?.rows ?? null;
            const delta = s.rows != null && prevRows != null ? s.rows - prevRows : null;
            const date = parseTime(v.timestamp_millis);
            const status = versionStatus(v.version, currentVersion);
            return (
              <tr
                key={v.version}
                {...rowProps(v.version)}
                aria-selected={v.version === active}
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
                <td className="text-right">
                  <Delta value={delta} />
                </td>
                <td className="text-right">{formatBytes(s.dataBytes)}</td>
                <td className="text-right">{formatCount(s.dataFiles)}</td>
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
              </tr>
            );
          })}
        </tbody>
      </ScrollTable>
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
  const [stickyTop, setStickyTop] = useState(0);
  const [view, setView] = useState<PaneView>("details");
  const all = versions.versions;
  const { current, index, close, rowProps } = usePaneSelection(all, versionOf);
  const ready = !versions.isPending && all.length > 0;
  // The table header sticks just below the sticky chart, whose height follows the viewport.
  useEffect(() => {
    const el = trends.current;
    if (!el) return;
    const observer = new ResizeObserver(() => setStickyTop(el.offsetHeight));
    observer.observe(el);
    return () => observer.disconnect();
  }, [ready]);

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
  return (
    <div className="@container" style={{ "--sticky-top": `${stickyTop}px` } as React.CSSProperties}>
      <div ref={trends} className="sticky top-(--tabs-h,0px) z-20 -mx-1 bg-background px-1 pt-1 pb-4">
        <TrendsSection loaded={all} />
      </div>
      <RefsTiles segments={segments} />
      <SidePaneLayout
        pane={
          current && (
            <VersionDetailPane
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
              top={stickyTop}
            />
          )
        }
      >
        <VersionHistory
          versions={versions}
          currentVersion={currentVersion}
          active={current?.version ?? null}
          rowProps={rowProps}
        />
      </SidePaneLayout>
    </div>
  );
}
