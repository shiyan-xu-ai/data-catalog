import type { UseQueryResult } from "@tanstack/react-query";
import { BarChart3, Database, FileStack, Layers, ListTree, Rows3, Trash2 } from "lucide-react";
import { formatBytes, formatCount } from "@/lib/format";
import {
  parseManifestSummary,
  topColumns,
  totalIndexBytes,
  versionSeries,
} from "@/lib/table-metadata";
import type { ColumnStorageResponse, DescribeTableResponse, IndexContent, TableVersion } from "@/lib/schemas";
import { Skeleton } from "@/components/ui/skeleton";
import { EmptyState } from "@/components/empty-state";
import { ErrorState } from "@/components/error-state";
import { Section } from "@/components/section";
import { ColumnStorageChart, GrowthChart, LegendItem, StorageSplitChart, type StorageSlice } from "./charts";

const TOP_COLUMNS = 12;

function Kpi({
  icon: Icon,
  label,
  value,
  sub,
  loading,
  failed,
}: {
  icon: typeof Rows3;
  label: string;
  value: string;
  sub?: string;
  loading?: boolean;
  failed?: boolean;
}) {
  return (
    <div className="rounded-lg border bg-card p-4 shadow-xs">
      <div className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
        <Icon className="h-3.5 w-3.5" /> {label}
      </div>
      {loading ? (
        <>
          <Skeleton className="mt-2 h-7 w-24" />
          <Skeleton className="mt-1.5 h-3 w-16" />
        </>
      ) : failed ? (
        <div className="mt-2 text-sm text-muted-foreground">Unavailable</div>
      ) : (
        <>
          <div className="mt-1 text-2xl font-semibold tracking-tight tabular-nums">{value}</div>
          <div className="mt-0.5 h-4 truncate text-xs text-muted-foreground tabular-nums">{sub}</div>
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
}: {
  detail: UseQueryResult<DescribeTableResponse>;
  versions: UseQueryResult<TableVersion[]>;
  indices: UseQueryResult<IndexContent[]>;
  columns: UseQueryResult<ColumnStorageResponse>;
}) {
  const s = parseManifestSummary(detail.data?.metadata);
  const indexBytes = indices.data ? totalIndexBytes(indices.data) : null;
  const series = versions.data ? versionSeries(versions.data) : [];

  return (
    <div className="@container space-y-6">
      {detail.error && <ErrorState message={`Lance metadata unavailable: ${detail.error.message}`} />}
      <div className="grid grid-cols-2 gap-3 @xl:grid-cols-3 @5xl:grid-cols-6">
        <Kpi
          icon={Rows3}
          label="Rows"
          value={formatCount(s.rows)}
          sub={s.fragments != null ? `${formatCount(s.fragments)} fragments` : undefined}
          loading={detail.isPending}
          failed={!!detail.error}
        />
        <Kpi
          icon={Database}
          label="Data size"
          value={formatBytes(s.dataBytes)}
          sub="data files on disk"
          loading={detail.isPending}
          failed={!!detail.error}
        />
        <Kpi
          icon={FileStack}
          label="Data files"
          value={formatCount(s.dataFiles)}
          loading={detail.isPending}
          failed={!!detail.error}
        />
        <Kpi
          icon={Trash2}
          label="Deleted rows"
          value={formatCount(s.deletedRows)}
          sub={s.deletionFiles != null ? `${formatCount(s.deletionFiles)} deletion files` : undefined}
          loading={detail.isPending}
          failed={!!detail.error}
        />
        <Kpi
          icon={Layers}
          label="Versions"
          value={formatCount(versions.data?.length)}
          sub={detail.data?.version != null ? `current v${detail.data.version}` : undefined}
          loading={versions.isPending}
          failed={!!versions.error}
        />
        <Kpi
          icon={ListTree}
          label="Indices"
          value={formatCount(indices.data?.length)}
          sub={indexBytes != null ? `${formatBytes(indexBytes)} on disk` : undefined}
          loading={indices.isPending}
          failed={!!indices.error}
        />
      </div>

      <div className="grid gap-6 @4xl:grid-cols-[minmax(0,3fr)_minmax(0,2fr)]">
        <Section
          title="Storage by column"
          description={
            columns.data
              ? `Bytes on disk per top-level column at version ${columns.data.version}`
              : "Bytes on disk per top-level column"
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

        <Section title="Storage composition" description="Data and index files, from manifest and index sizes">
          {detail.error ? (
            <ErrorState message={detail.error.message} />
          ) : detail.isPending || indices.isPending ? (
            <ChartSkeleton />
          ) : (
            <StorageComposition
              dataBytes={s.dataBytes}
              dataFiles={s.dataFiles}
              indexBytes={indexBytes}
              indexCount={indices.data?.length ?? null}
              indicesFailed={!!indices.error}
              deletionFiles={s.deletionFiles}
              deletedRows={s.deletedRows}
            />
          )}
        </Section>
      </div>

      <Section title="Growth" description="Rows and data size at every version">
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
              <span>{formatCount(series.length)} versions</span>
            </div>
          </>
        )}
      </Section>
    </div>
  );
}

/** Data vs index bytes. Deletion files carry no byte size in the manifest, so they are shown
 *  as counts only rather than as an estimated slice. */
function StorageComposition({
  dataBytes,
  dataFiles,
  indexBytes,
  indexCount,
  indicesFailed,
  deletionFiles,
  deletedRows,
}: {
  dataBytes: number | null;
  dataFiles: number | null;
  indexBytes: number | null;
  indexCount: number | null;
  indicesFailed: boolean;
  deletionFiles: number | null;
  deletedRows: number | null;
}) {
  const slices: StorageSlice[] = [];
  if (dataBytes != null) slices.push({ key: "data", label: "Data files", bytes: dataBytes, color: "hsl(var(--chart-1))" });
  if (indexBytes != null) slices.push({ key: "index", label: "Index files", bytes: indexBytes, color: "hsl(var(--chart-3))" });

  const indexNote = indicesFailed
    ? "Index list unavailable"
    : indexCount === 0
      ? "No indices"
      : indexBytes == null
        ? "Index sizes not reported"
        : `${formatCount(indexCount)} ${indexCount === 1 ? "index" : "indices"}`;

  return (
    <div className="space-y-4">
      {slices.every((x) => x.bytes === 0) ? (
        <EmptyState icon={Database} title="No sizes recorded">The manifest records no file sizes.</EmptyState>
      ) : (
        <StorageSplitChart slices={slices.filter((x) => x.bytes > 0)} />
      )}
      <div className="space-y-2">
        <LegendItem
          color="hsl(var(--chart-1))"
          label="Data files"
          value={formatBytes(dataBytes)}
          note={dataFiles != null ? `${formatCount(dataFiles)} files` : undefined}
        />
        <LegendItem color="hsl(var(--chart-3))" label="Index files" value={formatBytes(indexBytes)} note={indexNote} />
        <LegendItem
          color="hsl(var(--muted-foreground) / 0.4)"
          label="Deletion files"
          value={`${formatCount(deletionFiles)} files`}
          note={`${formatCount(deletedRows)} deleted rows · size not recorded`}
        />
      </div>
    </div>
  );
}
