import { useEffect, useRef, useState } from "react";
import { BarChart3, Layers } from "lucide-react";
import type { PagedVersions } from "@/lib/catalog-queries";
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
function TrendsSection({ loaded, complete }: { loaded: readonly TableVersion[]; complete: boolean }) {
  const series = versionSeries(loaded);
  const first = series[0]?.version;
  const last = series[series.length - 1]?.version;
  const range = first === last ? `v${first}` : `v${first}–v${last}`;
  const count = complete ? `all ${formatCount(series.length)} versions` : `${formatCount(series.length)} loaded`;
  return (
    <Section
      title="Trends"
      description={`Rows and data size, ${range} (${count})`}
      help="Covers the loaded versions only; load earlier versions below to extend it. Data size counts the data files each version references, the same measure as Current version data."
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

// Sticky just under the Trends chart; the 1px overlap hides sub-pixel seams at fractional zoom.
const HEAD = "sticky top-[calc(var(--trends-h)-1px)] z-10 h-10 bg-muted px-2 text-left align-middle font-medium whitespace-nowrap text-muted-foreground shadow-[inset_0_-1px_0_hsl(var(--border))]";

/** Loaded versions, newest first, with "Load earlier" for the next page. */
function VersionHistory({ versions, currentVersion }: { versions: PagedVersions; currentVersion: number | null | undefined }) {
  const formatDateTime = useDateTimeFormatter();
  const all = versions.versions;
  return (
    <Section
      title="Version history"
      description={`${formatCount(all.length)} ${versions.hasNextPage ? "loaded, newest first" : "versions, newest first"}`}
      help="Each version's figures come from its manifest summary. Δ Rows compares with the next older loaded version. Current is the version the catalog serves to reads that name no version; newer versions become current after a catalog refresh."
      contentClassName="px-0 pb-0"
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
            return (
              <tr key={v.version} className="transition-colors hover:bg-muted/50">
                <td className="pl-4">
                  <div className="flex items-center gap-2">
                    <span className="font-mono font-medium">v{v.version}</span>
                    {versionStatus(v.version, currentVersion) === "served" && <Badge variant="secondary">current</Badge>}
                    {versionStatus(v.version, currentVersion) === "newer" && <Badge variant="outline">newer than served</Badge>}
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

/** Trends over the loaded versions (sticky while the table scrolls), then the version table. */
export function VersionsTab({
  versions,
  currentVersion,
}: {
  versions: PagedVersions;
  currentVersion: number | null | undefined;
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
    <div style={{ "--trends-h": `${trendsHeight}px` } as React.CSSProperties}>
      <div ref={trends} className="sticky top-0 z-20 -mx-1 bg-background px-1 pt-1 pb-4">
        <TrendsSection loaded={versions.versions} complete={!versions.hasNextPage} />
      </div>
      <VersionHistory versions={versions} currentVersion={currentVersion} />
    </div>
  );
}
