import { useEffect, useRef, useState } from "react";
import type { UseQueryResult } from "@tanstack/react-query";
import { BarChart3, Layers } from "lucide-react";
import { useTableBranches, useTableTags, type PagedVersions } from "@/lib/catalog-queries";
import { formatBytes, formatCount, formatRelative, parseTime } from "@/lib/format";
import { useDateTimeFormatter } from "@/lib/preferences";
import { parseManifestSummary, versionSeries, versionStatus } from "@/lib/table-metadata";
import type { TableVersion } from "@/lib/schemas";
import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { EmptyState } from "@/components/empty-state";
import { ErrorState } from "@/components/error-state";
import { Section } from "@/components/section";
import { GrowthChart } from "./charts";

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

// Sticky just under the Trends chart; the 1px overlap hides sub-pixel seams at fractional zoom.
const HEAD = "sticky top-[calc(var(--trends-h)-1px)] z-10 h-10 bg-muted px-2 text-left align-middle font-medium whitespace-nowrap text-muted-foreground shadow-[inset_0_-1px_0_hsl(var(--border))]";

/** Loaded versions, newest first, with "Load earlier" for the next page. */
function VersionHistory({ versions, servedVersion }: { versions: PagedVersions; servedVersion: number | null | undefined }) {
  const formatDateTime = useDateTimeFormatter();
  const all = versions.versions;
  return (
    <Section
      title="Version history"
      description={versions.hasNextPage ? `${formatCount(all.length)} loaded` : undefined}
      help="Current marks the served version; newer versions need a catalog refresh."
    >
      {/* A plain table (no scroll wrapper) so its header can stick below the Trends chart. */}
      <table className="w-full text-sm">
        <thead>
          <tr>
            <th className={cn(HEAD, "pl-4")}>Version</th>
            <th className={HEAD}>Created</th>
            <th className={cn(HEAD, "text-right")}>Rows</th>
            <th className={cn(HEAD, "text-right")}>Δ Rows</th>
            <th className={cn(HEAD, "text-right")}>Data size</th>
            <th className={cn(HEAD, "text-right")}>Data files</th>
            <th className={cn(HEAD, "pr-4 text-right")}>Deletions</th>
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
            const status = versionStatus(v.version, servedVersion);
            return (
              <tr key={v.version} className="transition-colors hover:bg-muted/50">
                <td className="pl-4">
                  <div className="flex items-center gap-2">
                    <span className="font-mono font-medium">v{v.version}</span>
                    {status === "served" && <Badge variant="secondary">current</Badge>}
                    {status === "newer" && <Badge variant="outline">newer than served</Badge>}
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
 *  the version table. */
export function VersionsTab({
  segments,
  versions,
  servedVersion,
}: {
  segments: readonly string[];
  versions: PagedVersions;
  servedVersion: number | null | undefined;
}) {
  const trends = useRef<HTMLDivElement>(null);
  const [trendsHeight, setTrendsHeight] = useState(0);
  const ready = !versions.isPending && versions.versions.length > 0;
  // The table header sticks just below the sticky chart, whose height follows the viewport.
  useEffect(() => {
    const el = trends.current;
    if (!el) return;
    const observer = new ResizeObserver(() => setTrendsHeight(el.offsetHeight));
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
  if (versions.versions.length === 0) {
    if (versions.error) return <ErrorState message={`Versions unavailable: ${versions.error.message}`} />;
    return <EmptyState icon={Layers} title="No versions">The table reports no versions.</EmptyState>;
  }
  return (
    <div className="@container" style={{ "--trends-h": `${trendsHeight}px` } as React.CSSProperties}>
      <div ref={trends} className="sticky top-0 z-20 -mx-1 bg-background px-1 pt-1 pb-4">
        <TrendsSection loaded={versions.versions} />
      </div>
      <RefsTiles segments={segments} />
      <VersionHistory versions={versions} servedVersion={servedVersion} />
    </div>
  );
}
