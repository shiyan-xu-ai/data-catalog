import type { UseQueryResult } from "@tanstack/react-query";
import { BarChart3, CircleDollarSign, Database, HardDrive, ListTree, Rows3, Trash2 } from "lucide-react";
import { formatBytes, formatCost, formatCount, formatCountOf, formatGigabytes, formatPercent, formatRate } from "@/lib/format";
import {
  COMPACTION_TARGET_ROWS,
  compactionTargetOverride,
  parseManifestSummary,
  topColumns,
  totalIndexBytes,
} from "@/lib/table-metadata";
import type {
  ColumnStorageResponse,
  DescribeTableResponse,
  IndexContent,
  TableStatsResponse,
  TableStorageResponse,
} from "@/lib/schemas";
import { Skeleton } from "@/components/ui/skeleton";
import { HelpTooltip } from "@/components/help-tooltip";
import { EmptyState } from "@/components/empty-state";
import { ErrorState } from "@/components/error-state";
import { Section } from "@/components/section";
import { ColumnStorageChart, LegendItem, StorageSplitChart, type StorageSlice } from "./charts";
import { FragmentDistributionChart, SmallFragmentShare } from "./fragment-charts";

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
  indices,
  columns,
  storage,
  stats,
}: {
  detail: UseQueryResult<DescribeTableResponse>;
  indices: UseQueryResult<IndexContent[]>;
  columns: UseQueryResult<ColumnStorageResponse>;
  storage: UseQueryResult<TableStorageResponse>;
  stats: UseQueryResult<TableStatsResponse>;
}) {
  const s = parseManifestSummary(detail.data?.metadata);
  const indexBytes = indices.data ? totalIndexBytes(indices.data) : null;
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
          sub={s.fragments != null ? formatCountOf(s.fragments, "fragment") : undefined}
          help="Live rows in the current version (deleted rows excluded), from the manifest summary."
          loading={detail.isPending}
          failed={detail.error?.message}
        />
        <Kpi
          icon={Database}
          label="Current version data"
          value={formatBytes(s.dataBytes)}
          sub={s.dataFiles != null ? formatCountOf(s.dataFiles, "data file") : undefined}
          help="Size and count of the data files the current version's fragments reference, from the manifest summary. Excludes blob files, index files, and files kept only by older versions."
          loading={detail.isPending}
          failed={detail.error?.message}
        />
        <Kpi
          icon={Trash2}
          label="Deleted rows"
          value={formatCount(s.deletedRows)}
          sub={s.deletionFiles != null ? formatCountOf(s.deletionFiles, "deletion file") : undefined}
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
          help="Size of the current version's index files. Index files kept only by older versions count toward Index files in Storage composition."
          loading={indices.isPending}
          failed={indices.error?.message}
        />
        <Kpi
          icon={HardDrive}
          label="Total on storage"
          value={formatBytes(onStorage?.total_bytes)}
          sub={onStorage ? formatCountOf(onStorage.total_objects, "object") : undefined}
          help={
            <>
              Size of every object under the table location, across all versions
              {onStorage ? ` (${formatGigabytes(onStorage.total_bytes)})` : ""}. The catalog lists the table location to
              measure it. Includes blob files, index files, deletion files, manifests, and transactions.
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
          description="Bytes on disk per column, current version"
          help={
            <>
              Bytes on disk per top-level column in the current version&apos;s data files, from Lance field-level data
              statistics.
              {stats.data
                ? ` These statistics total ${formatBytes(stats.data.total_bytes)}. The manifest data size, ${formatBytes(s.dataBytes)}, also includes file metadata.`
                : ""}{" "}
              Counts scalar and inline column data only. Excludes blob files (data/*.blob), which hold most storage, and
              index files. A blob column such as log_scenario counts only its per-row descriptors, which point into .blob
              files.
            </>
          }
        >
          {columns.error ? (
            <ErrorState message={columns.error.message} />
          ) : !columns.data ? (
            <ChartSkeleton height="h-64" caption="Measuring column sizes across every fragment. Large tables take several seconds." />
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
            <ChartSkeleton caption="Listing objects under the table location. Large tables take several seconds." />
          ) : (
            <StorageComposition onStorage={storage.data.on_storage} />
          )}
        </Section>
      </div>

      <Section
        title="Fragment distribution"
        description="Rows per fragment, current version"
        help={
          <>
            Lance GetTableStats reports these figures. Rows per fragment are physical rows and include deleted rows. A
            fragment is small when it has fewer rows than Lance&apos;s default compaction target,{" "}
            {formatCount(COMPACTION_TARGET_ROWS)} rows (target_rows_per_fragment in CompactionOptions::default). A table
            can override the target with lance.compaction.target_rows_per_fragment. GetTableStats still counts small
            fragments against the default.
          </>
        }
      >
        {stats.error ? (
          <ErrorState message={stats.error.message} />
        ) : !stats.data ? (
          <ChartSkeleton height="h-28" caption="Reading fragment statistics. Large tables take several seconds." />
        ) : stats.data.fragment_stats.num_fragments === 0 ? (
          <EmptyState icon={BarChart3} title="No fragments">The current version has no data fragments.</EmptyState>
        ) : (
          <div className="grid gap-6 @3xl:grid-cols-[minmax(0,2fr)_minmax(0,1fr)]">
            <FragmentDistributionChart lengths={stats.data.fragment_stats.lengths} />
            <div className="border-t pt-5 @3xl:border-t-0 @3xl:border-l @3xl:pt-0 @3xl:pl-6">
              <SmallFragmentShare
                small={stats.data.fragment_stats.num_small_fragments}
                total={stats.data.fragment_stats.num_fragments}
                targetOverride={compactionTargetOverride(detail.data?.metadata)}
              />
            </div>
          </div>
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
  // Synthetic non-blob total for the blob split; neutral so it is not mistaken for a category.
  others: { label: "Everything else", color: "hsl(var(--foreground) / 0.35)" },
};
const ORDER = Object.keys(CATEGORIES);

type StorageCategory = TableStorageResponse["on_storage"]["categories"][number];

/** Donut plus legend for a set of categories; shares are against `total`. */
function CategoryBreakdown({
  title,
  categories,
  total,
  caption,
  shareSuffix = "",
}: {
  title: string;
  categories: StorageCategory[];
  total: number;
  caption: string;
  shareSuffix?: string;
}) {
  const style = (name: string) => CATEGORIES[name] ?? { label: name, color: "hsl(var(--muted-foreground) / 0.5)" };
  const slices: StorageSlice[] = categories
    .filter((c) => c.bytes > 0)
    .map((c) => ({ key: c.name, label: style(c.name).label, bytes: c.bytes, color: style(c.name).color }));
  return (
    <div className="space-y-4">
      <p className="text-xs font-medium text-muted-foreground">{title}</p>
      <StorageSplitChart slices={slices} caption={caption} className="h-44" />
      <div className="space-y-2">
        {categories.map((c) => (
          <LegendItem
            key={c.name}
            color={style(c.name).color}
            label={style(c.name).label}
            value={formatBytes(c.bytes)}
            note={`${formatCountOf(c.objects, "object")} · ${formatPercent(total ? c.bytes / total : null)}${shareSuffix}`}
          />
        ))}
      </div>
    </div>
  );
}

/** With blob files: blob vs everything else, then the non-blob remainder by category, since
 *  blob payloads dwarf every other category. Without them: one donut by category. */
function StorageComposition({ onStorage }: { onStorage: TableStorageResponse["on_storage"] }) {
  if (onStorage.total_bytes === 0) {
    return <EmptyState icon={HardDrive} title="Nothing on storage">No objects were found under the table location.</EmptyState>;
  }
  const rank = (name: string) => (ORDER.includes(name) ? ORDER.indexOf(name) : ORDER.length);
  const categories = [...onStorage.categories].sort((a, b) => rank(a.name) - rank(b.name));
  const blob = categories.find((c) => c.name === "blob");
  const rest = categories.filter((c) => c.name !== "blob");

  if (!blob || blob.bytes === 0) {
    return <CategoryBreakdown title="All objects" categories={rest} total={onStorage.total_bytes} caption="on storage" />;
  }

  const restBytes = rest.reduce((s, c) => s + c.bytes, 0);
  const restObjects = rest.reduce((s, c) => s + c.objects, 0);
  return (
    <div className="grid gap-6 @lg:grid-cols-2">
      <CategoryBreakdown
        title="All objects"
        categories={[blob, { name: "others", bytes: restBytes, objects: restObjects }]}
        total={onStorage.total_bytes}
        caption="on storage"
      />
      <CategoryBreakdown
        title="Excluding blob files"
        categories={rest}
        total={restBytes}
        caption="non-blob"
        shareSuffix=" of non-blob"
      />
    </div>
  );
}
