import type { UseQueryResult } from "@tanstack/react-query";
import { BarChart3, CircleDollarSign, Database, HardDrive, ListTree, Rows3, Trash2 } from "lucide-react";
import { formatBytes, formatCost, formatCount, formatGigabytes, formatPercent, formatRate } from "@/lib/format";
import { COMPACTION_TARGET_ROWS, parseManifestSummary, topColumns, totalIndexBytes, versionSeries } from "@/lib/table-metadata";
import type {
  ColumnStorageResponse,
  DescribeTableResponse,
  IndexContent,
  TableStatsResponse,
  TableStorageResponse,
  TableVersion,
} from "@/lib/schemas";
import { Skeleton } from "@/components/ui/skeleton";
import { HelpTooltip } from "@/components/help-tooltip";
import { EmptyState } from "@/components/empty-state";
import { ErrorState } from "@/components/error-state";
import { Section } from "@/components/section";
import { ColumnStorageChart, GrowthChart, LegendItem, StorageSplitChart, type StorageSlice } from "./charts";
import { FragmentBoxPlot, SmallFragmentShare } from "./fragment-charts";

const TOP_COLUMNS = 12;

function Kpi({
  icon: Icon,
  label,
  value,
  sub,
  help,
  loading,
  failed,
}: {
  icon: typeof Rows3;
  label: string;
  value: string;
  /** Short caption under the figure. */
  sub?: string;
  /** What the figure measures and excludes, behind a (?) icon. */
  help?: React.ReactNode;
  loading?: boolean;
  /** Error message; the card shows "Unavailable" with the message on hover. */
  failed?: string;
}) {
  return (
    <div className="rounded-lg border bg-card p-4 shadow-xs">
      <div className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
        <Icon className="h-3.5 w-3.5" /> {label}
        {help && <HelpTooltip label={`About ${label}`}>{help}</HelpTooltip>}
      </div>
      {loading ? (
        <>
          <Skeleton className="mt-2 h-7 w-24" />
          <Skeleton className="mt-1.5 h-3 w-16" />
        </>
      ) : failed ? (
        <div className="mt-2 text-sm text-muted-foreground" title={failed}>Unavailable</div>
      ) : (
        <>
          <div className="mt-1 text-2xl font-semibold tracking-tight tabular-nums">{value}</div>
          <div className="mt-0.5 min-h-4 truncate text-xs text-muted-foreground tabular-nums">{sub}</div>
        </>
      )}
    </div>
  );
}

function ChartSkeleton({ caption, height = "h-52" }: { caption?: string; height?: string }) {
  return (
    <div className="space-y-2">
      <Skeleton className={`${height} w-full`} />
      {caption && <p className="text-xs text-muted-foreground">{caption}</p>}
    </div>
  );
}

export function OverviewTab({
  detail,
  versions,
  indices,
  columns,
  storage,
  stats,
}: {
  detail: UseQueryResult<DescribeTableResponse>;
  versions: UseQueryResult<TableVersion[]>;
  indices: UseQueryResult<IndexContent[]>;
  columns: UseQueryResult<ColumnStorageResponse>;
  storage: UseQueryResult<TableStorageResponse>;
  stats: UseQueryResult<TableStatsResponse>;
}) {
  const s = parseManifestSummary(detail.data?.metadata);
  const indexBytes = indices.data ? totalIndexBytes(indices.data) : null;
  const series = versions.data ? versionSeries(versions.data) : [];
  // The storage listing is enabled lazily, so "not loaded yet" covers both idle and fetching.
  const storageLoading = !storage.data && !storage.error;
  const pricing = storage.data?.pricing;
  const onStorage = storage.data?.on_storage;

  return (
    <div className="@container space-y-6">
      {detail.error && <ErrorState message={`Lance metadata unavailable: ${detail.error.message}`} />}
      <div className="grid grid-cols-2 gap-3 @3xl:grid-cols-3">
        <Kpi
          icon={Rows3}
          label="Rows"
          value={formatCount(s.rows)}
          sub={s.fragments != null ? `${formatCount(s.fragments)} fragments` : undefined}
          help="Live rows in the current version (deleted rows excluded), from the manifest summary."
          loading={detail.isPending}
          failed={detail.error?.message}
        />
        <Kpi
          icon={Database}
          label="Current version data"
          value={formatBytes(s.dataBytes)}
          sub={s.dataFiles != null ? `${formatCount(s.dataFiles)} data files` : undefined}
          help="Size and count of the data files the current version's fragments reference, from the manifest summary. Excludes blob payload files, index files, and files kept only by older versions."
          loading={detail.isPending}
          failed={detail.error?.message}
        />
        <Kpi
          icon={Trash2}
          label="Deleted rows"
          value={formatCount(s.deletedRows)}
          sub={s.deletionFiles != null ? `${formatCount(s.deletionFiles)} deletion files` : undefined}
          help="Rows marked deleted by deletion files in the current version. They still occupy data files until compaction."
          loading={detail.isPending}
          failed={detail.error?.message}
        />
        <Kpi
          icon={ListTree}
          label="Index size"
          // No indices is a measured zero; indices without reported sizes stay unknown.
          value={formatBytes(indices.data?.length === 0 ? 0 : indexBytes)}
          sub="current version"
          help="Size of the current version's index files. Index files kept only by older versions are counted in Storage composition's Index files."
          loading={indices.isPending}
          failed={indices.error?.message}
        />
        <Kpi
          icon={HardDrive}
          label="Total on storage"
          value={formatBytes(onStorage?.total_bytes)}
          sub={onStorage ? `${formatCount(onStorage.total_objects)} objects` : undefined}
          help={
            <>
              Every object under the table location across all versions, measured by listing the location
              {onStorage ? ` (${formatGigabytes(onStorage.total_bytes)})` : ""}. Includes blob payloads, indices,
              deletion files, manifests, and transactions.
            </>
          }
          loading={storageLoading}
          failed={storage.error?.message}
        />
        <Kpi
          icon={CircleDollarSign}
          label="Est. monthly cost"
          value={pricing ? formatCost(pricing.estimated_monthly_cost, pricing.currency) : "—"}
          sub="storage only, estimate"
          help={
            <>
              Total on storage × {pricing ? formatRate(pricing.price_per_gb_month, pricing.currency) : "the configured rate"}
              {pricing ? ` (${pricing.tier})` : ""}. An upper bound if objects were auto-tiered to Infrequent. Excludes
              requests, retrieval, and egress.
            </>
          }
          loading={storageLoading}
          failed={storage.error?.message}
        />
      </div>

      <div className="grid gap-6 @4xl:grid-cols-[minmax(0,3fr)_minmax(0,2fr)]">
        <Section
          title="Storage by column"
          description="Current version, inline column data; excludes blob payloads"
          help={
            <>
              Bytes on disk per top-level column in the current version&apos;s data files (Lance field-level data
              statistics
              {stats.data
                ? `, total ${formatBytes(stats.data.total_bytes)} vs ${formatBytes(s.dataBytes)} manifest data size including file metadata`
                : ""}
              ). Scalar/inline column data only: excludes blob payload files (data/*.blob), which hold most storage, and
              index files. Blob columns such as log_scenario count only their per-row descriptors (pointers into .blob
              files).
            </>
          }
        >
          {columns.error ? (
            <ErrorState message={columns.error.message} />
          ) : !columns.data ? (
            <ChartSkeleton height="h-64" caption="Measuring column sizes across every fragment; large tables take several seconds." />
          ) : columns.data.columns.length === 0 ? (
            <EmptyState icon={BarChart3} title="No column storage">The current version has no data files.</EmptyState>
          ) : (
            <ColumnStorageChart columns={topColumns(columns.data.columns, TOP_COLUMNS)} />
          )}
        </Section>

        <Section title="Storage composition" description="Every object under the table location, by file category">
          {storage.error ? (
            <ErrorState message={storage.error.message} />
          ) : !storage.data ? (
            <ChartSkeleton caption="Listing every object under the table location; large tables take several seconds." />
          ) : (
            <StorageComposition onStorage={storage.data.on_storage} />
          )}
        </Section>
      </div>

      <Section
        title="Fragments"
        description="Physical rows per fragment in the current version"
        help={
          <>
            From Lance GetTableStats (POST /v1/table/&#123;id&#125;/stats). Fragment row counts are physical rows,
            including rows later marked deleted. A fragment is small when it has fewer than{" "}
            {formatCount(COMPACTION_TARGET_ROWS)} rows, Lance&apos;s default compaction target.
          </>
        }
      >
        {stats.error ? (
          <ErrorState message={stats.error.message} />
        ) : !stats.data ? (
          <ChartSkeleton height="h-28" caption="Reading fragment statistics; large tables take several seconds." />
        ) : stats.data.fragment_stats.num_fragments === 0 ? (
          <EmptyState icon={BarChart3} title="No fragments">The current version has no data fragments.</EmptyState>
        ) : (
          <div className="grid items-center gap-6 @4xl:grid-cols-[minmax(0,3fr)_minmax(0,2fr)]">
            <FragmentBoxPlot lengths={stats.data.fragment_stats.lengths} />
            <SmallFragmentShare
              small={stats.data.fragment_stats.num_small_fragments}
              total={stats.data.fragment_stats.num_fragments}
            />
          </div>
        )}
      </Section>

      <Section title="Trends" description="Rows and current-version data size at every version">
        {versions.error ? (
          <ErrorState message={versions.error.message} />
        ) : versions.isPending ? (
          <ChartSkeleton height="h-64" />
        ) : series.every((p) => p.rows == null && p.bytes == null) ? (
          <EmptyState icon={BarChart3} title="No version history">No version reports rows or size.</EmptyState>
        ) : (
          <>
            <GrowthChart points={series} />
            <div className="mt-3 flex flex-wrap justify-center gap-x-6 gap-y-1 text-xs text-muted-foreground">
              <span className="flex items-center gap-1.5">
                <span className="h-0.5 w-4 rounded bg-[hsl(var(--chart-1))]" /> Rows (left axis)
              </span>
              <span className="flex items-center gap-1.5">
                <span className="h-0 w-4 border-t-2 border-dashed border-[hsl(var(--chart-2))]" /> Data size (right axis)
              </span>
            </div>
          </>
        )}
      </Section>
    </div>
  );
}

// Display order and colors for the route's categories; unknown names fall into the muted color.
const CATEGORIES: Record<string, { label: string; color: string }> = {
  data: { label: "Data files", color: "hsl(var(--chart-1))" },
  blob: { label: "Blob files", color: "hsl(var(--chart-4))" },
  index: { label: "Index files", color: "hsl(var(--chart-3))" },
  deletion: { label: "Deletion files", color: "hsl(var(--chart-5))" },
  manifest: { label: "Manifests", color: "hsl(var(--chart-2))" },
  transaction: { label: "Transactions", color: "hsl(var(--chart-6))" },
  other: { label: "Other", color: "hsl(var(--muted-foreground) / 0.5)" },
};
const ORDER = Object.keys(CATEGORIES);

function StorageComposition({ onStorage }: { onStorage: TableStorageResponse["on_storage"] }) {
  const rank = (name: string) => (ORDER.includes(name) ? ORDER.indexOf(name) : ORDER.length);
  const categories = [...onStorage.categories].sort((a, b) => rank(a.name) - rank(b.name));
  const style = (name: string) => CATEGORIES[name] ?? { label: name, color: "hsl(var(--muted-foreground) / 0.5)" };
  const slices: StorageSlice[] = categories
    .filter((c) => c.bytes > 0)
    .map((c) => ({ key: c.name, label: style(c.name).label, bytes: c.bytes, color: style(c.name).color }));

  return (
    <div className="space-y-4">
      {slices.length === 0 ? (
        <EmptyState icon={HardDrive} title="Nothing on storage">No objects were found under the table location.</EmptyState>
      ) : (
        <StorageSplitChart slices={slices} caption="on storage" />
      )}
      <div className="space-y-2">
        {categories.map((c) => (
          <LegendItem
            key={c.name}
            color={style(c.name).color}
            label={style(c.name).label}
            value={formatBytes(c.bytes)}
            note={`${formatCount(c.objects)} objects · ${formatPercent(onStorage.total_bytes ? c.bytes / onStorage.total_bytes : null)}`}
          />
        ))}
      </div>
    </div>
  );
}
