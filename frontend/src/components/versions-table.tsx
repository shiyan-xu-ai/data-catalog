import { useState } from "react";
import { Link } from "@tanstack/react-router";
import { ChevronLeft, ChevronRight } from "lucide-react";
import type { TableVersion } from "@/lib/schemas";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ShapeBadge } from "@/components/shape-badge";
import { formatBytes, formatCount, formatTime } from "@/lib/format";
import { useTz } from "@/lib/timezone";

interface Props {
  tableId: string;
  versions: TableVersion[];
}

export function VersionsTable({ tableId, versions }: Props) {
  const { mode: tz } = useTz();
  const [page, setPage] = useState(0);

  const pageSize = 20;
  const sorted = [...versions].sort(
    (a, b) => b.timestamp.localeCompare(a.timestamp) || b.version_id.localeCompare(a.version_id),
  );
  const pageCount = Math.max(1, Math.ceil(sorted.length / pageSize));
  const currentPage = Math.min(page, pageCount - 1);
  const pageVersions = sorted.slice(currentPage * pageSize, currentPage * pageSize + pageSize);

  return (
    <div className="space-y-3">
      <div className="overflow-x-auto">
        <table className="w-full text-sm">
          <thead className="border-b bg-muted/40 text-left text-xs uppercase tracking-wide text-muted-foreground whitespace-nowrap">
            <tr>
              <th className="px-3 py-2">Version</th>
              <th className="px-3 py-2">Shape</th>
              <th className="px-3 py-2 text-right">Size</th>
              <th className="px-3 py-2 text-right">Rows</th>
              <th className="px-3 py-2">Synced</th>
            </tr>
          </thead>
          <tbody>
            {pageVersions.map((v) => (
              <tr key={v.version_id} className="border-b last:border-0 hover:bg-muted/40">
                <td className="px-3 py-2">
                  <Link
                    to="/tables/$tableId/versions/$versionId"
                    params={{ tableId, versionId: v.version_id }}
                    className="font-mono text-xs text-primary hover:underline"
                  >
                    {v.version_id}
                  </Link>
                  {v.partial && <Badge variant="warning" className="ml-2">partial</Badge>}
                </td>
                <td className="px-3 py-2"><ShapeBadge shape={v.shape} /></td>
                <td className="px-3 py-2 text-right tabular-nums text-muted-foreground">{formatBytes(v.storage_bytes_total)}</td>
                <td className="px-3 py-2 text-right tabular-nums text-muted-foreground">{formatCount(v.row_count)}</td>
                <td className="px-3 py-2 whitespace-nowrap text-muted-foreground">{formatTime(v.synced_at, tz)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {versions.length > pageSize && (
        <div className="flex items-center justify-between text-sm">
          <span className="text-muted-foreground">
            Page {currentPage + 1} of {pageCount}
          </span>
          <div className="flex gap-2">
            <Button
              size="sm"
              variant="outline"
              onClick={() => setPage((p) => Math.max(0, p - 1))}
              disabled={currentPage <= 0}
            >
              <ChevronLeft className="h-4 w-4" /> Prev
            </Button>
            <Button
              size="sm"
              variant="outline"
              onClick={() => setPage((p) => Math.min(pageCount - 1, p + 1))}
              disabled={currentPage >= pageCount - 1}
            >
              Next <ChevronRight className="h-4 w-4" />
            </Button>
          </div>
        </div>
      )}
    </div>
  );
}
