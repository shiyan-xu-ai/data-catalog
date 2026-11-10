import { useState } from "react";
import type { UseQueryResult } from "@tanstack/react-query";
import { ChevronLeft, ChevronRight, Layers } from "lucide-react";
import { formatBytes, formatCount, formatRelative, parseTime } from "@/lib/format";
import { useDateTimeFormatter } from "@/lib/preferences";
import { parseManifestSummary } from "@/lib/table-metadata";
import type { TableVersion } from "@/lib/schemas";
import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { EmptyState } from "@/components/empty-state";
import { ErrorState } from "@/components/error-state";
import { Section } from "@/components/section";

const PAGE_SIZE = 20;

function Delta({ value }: { value: number | null }) {
  if (value == null || value === 0) return <span className="text-muted-foreground/60">—</span>;
  return (
    <span className={cn(value > 0 ? "text-emerald-600 dark:text-emerald-400" : "text-rose-600 dark:text-rose-400")}>
      {value > 0 ? "+" : "−"}
      {formatCount(Math.abs(value))}
    </span>
  );
}

/** Every version, newest first, paged client-side (the full list also feeds the growth chart). */
export function VersionsTab({
  versions,
  currentVersion,
}: {
  versions: UseQueryResult<TableVersion[]>;
  currentVersion: number | null | undefined;
}) {
  const [page, setPage] = useState(0);
  const formatDateTime = useDateTimeFormatter();
  if (versions.error) return <ErrorState message={`Versions unavailable: ${versions.error.message}`} />;
  if (versions.isPending) {
    return (
      <div className="space-y-2">
        {Array.from({ length: 8 }, (_, i) => (
          <Skeleton key={i} className="h-9 w-full" />
        ))}
      </div>
    );
  }
  const all = versions.data;
  if (all.length === 0) return <EmptyState icon={Layers} title="No versions">The table reports no versions.</EmptyState>;

  const pages = Math.ceil(all.length / PAGE_SIZE);
  const current = Math.min(page, pages - 1);
  const start = current * PAGE_SIZE;
  const visible = all.slice(start, start + PAGE_SIZE);

  return (
    <Section
      title="Version history"
      description={`${formatCount(all.length)} versions; per-version figures come from each manifest summary`}
      contentClassName="px-0 pb-0"
    >
      <Table>
        <TableHeader className="bg-muted/40">
          <TableRow className="hover:bg-transparent">
            <TableHead className="pl-4">Version</TableHead>
            <TableHead>Created</TableHead>
            <TableHead className="text-right">Rows</TableHead>
            <TableHead className="text-right">Δ Rows</TableHead>
            <TableHead className="text-right">Data size</TableHead>
            <TableHead className="text-right">Data files</TableHead>
            <TableHead className="pr-4 text-right">Deletions</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody className="tabular-nums [&_td]:whitespace-nowrap">
          {visible.map((v, i) => {
            const s = parseManifestSummary(v.metadata);
            // `all` is newest first, so the previous version is the next entry.
            const prev = all[start + i + 1];
            const prevRows = prev ? parseManifestSummary(prev.metadata).rows : null;
            const delta = s.rows != null && prevRows != null ? s.rows - prevRows : null;
            const date = parseTime(v.timestamp_millis);
            return (
              <TableRow key={v.version}>
                <TableCell className="pl-4">
                  <div className="flex items-center gap-2">
                    <span className="font-mono font-medium">v{v.version}</span>
                    {v.version === currentVersion && <Badge variant="secondary">current</Badge>}
                  </div>
                </TableCell>
                <TableCell className="whitespace-nowrap">
                  <div>{formatRelative(date)}</div>
                  <div className="text-xs text-muted-foreground">{formatDateTime(date)}</div>
                </TableCell>
                <TableCell className="text-right">{formatCount(s.rows)}</TableCell>
                <TableCell className="text-right">
                  <Delta value={delta} />
                </TableCell>
                <TableCell className="text-right">{formatBytes(s.dataBytes)}</TableCell>
                <TableCell className="text-right">{formatCount(s.dataFiles)}</TableCell>
                <TableCell className="pr-4 text-right whitespace-nowrap">
                  {s.deletionFiles ? (
                    <>
                      {formatCount(s.deletedRows)} rows
                      <div className="text-xs text-muted-foreground">{formatCount(s.deletionFiles)} files</div>
                    </>
                  ) : (
                    <span className="text-muted-foreground/60">{s.deletionFiles === 0 ? "none" : "—"}</span>
                  )}
                </TableCell>
              </TableRow>
            );
          })}
        </TableBody>
      </Table>
      {pages > 1 && (
        <div className="flex items-center justify-between border-t px-4 py-2.5 text-xs text-muted-foreground">
          <span className="tabular-nums">
            {formatCount(start + 1)}–{formatCount(start + visible.length)} of {formatCount(all.length)}
          </span>
          <div className="flex items-center gap-1">
            <Button size="sm" variant="outline" className="h-7" disabled={current === 0} onClick={() => setPage(current - 1)}>
              <ChevronLeft /> Newer
            </Button>
            <span className="px-2 tabular-nums">
              Page {current + 1} / {pages}
            </span>
            <Button
              size="sm"
              variant="outline"
              className="h-7"
              disabled={current >= pages - 1}
              onClick={() => setPage(current + 1)}
            >
              Older <ChevronRight />
            </Button>
          </div>
        </div>
      )}
    </Section>
  );
}
