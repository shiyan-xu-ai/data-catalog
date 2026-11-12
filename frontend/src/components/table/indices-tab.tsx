import type { UseQueryResult } from "@tanstack/react-query";
import { ListTree } from "lucide-react";
import { formatBytes, formatCompact, formatCount, formatPercent } from "@/lib/format";
import { indexCoverage } from "@/lib/table-metadata";
import type { IndexContent } from "@/lib/schemas";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { TableBody, TableCell, TableRow } from "@/components/ui/table";
import { cn } from "@/lib/utils";
import { Dash, EmptyState } from "@/components/empty-state";
import { ErrorState } from "@/components/error-state";
import { Section } from "@/components/section";
import { Timestamp } from "@/components/timestamp";
import { Identifier } from "@/components/identifier";
import { HEAD_CELL, ScrollTable } from "./scroll-table";

function Coverage({ ix }: { ix: IndexContent }) {
  const fraction = indexCoverage(ix);
  if (fraction == null) return <Dash />;
  const total = (ix.num_indexed_rows ?? 0) + (ix.num_unindexed_rows ?? 0);
  return (
    <div className="w-44 space-y-1">
      <div
        className="flex items-baseline justify-between gap-2 text-xs whitespace-nowrap"
        title={`${formatCount(ix.num_indexed_rows)} of ${formatCount(total)} rows indexed`}
      >
        <span className="font-medium">{formatPercent(fraction)}</span>
        <span className="text-muted-foreground">
          {formatCompact(ix.num_indexed_rows)} / {formatCompact(total)}
        </span>
      </div>
      <div className="h-1.5 overflow-hidden rounded-full bg-muted">
        <div
          className={fraction < 1 ? "h-full rounded-full bg-amber-500" : "h-full rounded-full bg-emerald-500"}
          style={{ width: `${fraction * 100}%` }}
        />
      </div>
      {ix.num_unindexed_rows ? (
        <div className="text-[11px] text-muted-foreground">{formatCompact(ix.num_unindexed_rows)} rows unindexed</div>
      ) : null}
    </div>
  );
}

export function IndicesTab({ indices }: { indices: UseQueryResult<IndexContent[]> }) {
  if (indices.error) return <ErrorState message={`Indices unavailable: ${indices.error.message}`} />;
  if (indices.isPending) {
    return (
      <div className="space-y-2">
        {Array.from({ length: 4 }, (_, i) => (
          <Skeleton key={i} className="h-10 w-full" />
        ))}
      </div>
    );
  }
  const list = [...indices.data].sort((a, b) => a.index_name.localeCompare(b.index_name));
  if (list.length === 0) {
    return (
      <EmptyState icon={ListTree} title="No indices">
        The current version has no scalar or vector indices.
      </EmptyState>
    );
  }
  return (
    <Section
      title="Indices"
      help="Coverage is indexed rows out of indexed plus unindexed rows."
      contentClassName="px-0 pb-0"
    >
      <ScrollTable
        className="w-full text-sm"
        head={
          <tr>
            <th className={cn(HEAD_CELL, "min-w-48 pl-4")}>Name</th>
            <th className={HEAD_CELL}>Type</th>
            <th className={HEAD_CELL}>Columns</th>
            <th className={cn(HEAD_CELL, "text-right")}>Size</th>
            <th className={cn(HEAD_CELL, "w-48 pr-4")}>Coverage</th>
          </tr>
        }
      >
        <TableBody className="tabular-nums [&_td]:px-2 [&_td]:py-2 [&_td]:align-top">
          {list.map((ix) => (
            <TableRow key={ix.index_uuid ?? ix.index_name}>
              <TableCell className="pl-4">
                <Identifier name={ix.index_name} className="font-mono text-[13px] font-medium" />
                <div className="text-xs text-muted-foreground">
                  {ix.status}
                  {ix.status && ix.created_at && " · "}
                  {ix.created_at && (
                    <>
                      created <Timestamp value={ix.created_at} />
                    </>
                  )}
                </div>
              </TableCell>
              <TableCell>{ix.index_type ? <Badge variant="secondary">{ix.index_type}</Badge> : <Dash />}</TableCell>
              <TableCell>
                <div className="flex flex-wrap gap-1">
                  {ix.columns.map((c) => (
                    <Badge key={c} variant="outline" className="font-mono font-normal">
                      <Identifier name={c} />
                    </Badge>
                  ))}
                </div>
              </TableCell>
              <TableCell className="text-right whitespace-nowrap">{formatBytes(ix.size_bytes)}</TableCell>
              <TableCell className="pr-4">
                <Coverage ix={ix} />
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </ScrollTable>
    </Section>
  );
}
