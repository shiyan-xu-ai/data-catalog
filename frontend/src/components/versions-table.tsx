import { useState } from "react";
import { Link } from "@tanstack/react-router";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { Shield, ShieldOff, ChevronLeft, ChevronRight } from "lucide-react";
import { protectVersion } from "@/lib/api";
import type { TableVersion } from "@/lib/schemas";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { ShapeBadge } from "@/components/shape-badge";
import { formatBytes, formatTime } from "@/lib/format";
import { useTz } from "@/lib/timezone";

interface Props {
  tableId: string;
  versions: TableVersion[];
}

export function VersionsTable({ tableId, versions }: Props) {
  const qc = useQueryClient();
  const { mode: tz } = useTz();
  const [pending, setPending] = useState<string | null>(null);
  const [page, setPage] = useState(0);

  const pageSize = 20;
  const sorted = [...versions].sort(
    (a, b) => b.timestamp.localeCompare(a.timestamp) || b.version_id.localeCompare(a.version_id),
  );
  const pageCount = Math.max(1, Math.ceil(sorted.length / pageSize));
  const currentPage = Math.min(page, pageCount - 1);
  const pageVersions = sorted.slice(currentPage * pageSize, currentPage * pageSize + pageSize);

  const protectMut = useMutation({
    mutationFn: ({ vid, protect }: { vid: string; protect: boolean }) =>
      protectVersion(tableId, vid, protect),
    onMutate: ({ vid }) => setPending(vid),
    onSettled: () => setPending(null),
    onSuccess: (version) => {
      toast.success(version.protected ? `Protected ${version.version_id}` : `Unprotected ${version.version_id}`);
      qc.invalidateQueries({ queryKey: ["table", tableId] });
      qc.invalidateQueries({ queryKey: ["tables"] });
    },
    onError: (e: unknown) => toast.error((e as Error).message),
  });

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
              <th className="px-3 py-2">Swept</th>
              <th className="px-3 py-2 text-right">Protect</th>
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
                <td className="px-3 py-2 text-right tabular-nums text-muted-foreground">{v.row_count != null ? v.row_count.toLocaleString() : "—"}</td>
                <td className="px-3 py-2 whitespace-nowrap text-muted-foreground">{formatTime(v.swept_at, tz)}</td>
                <td className="px-3 py-2 text-right">
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <Button
                        size="icon"
                        variant={v.protected ? "default" : "ghost"}
                        disabled={pending === v.version_id}
                        onClick={() => protectMut.mutate({ vid: v.version_id, protect: !v.protected })}
                      >
                        {v.protected ? <Shield className="h-4 w-4" /> : <ShieldOff className="h-4 w-4" />}
                      </Button>
                    </TooltipTrigger>
                    <TooltipContent>{v.protected ? "Protected — click to unprotect" : "Click to protect from TTL"}</TooltipContent>
                  </Tooltip>
                </td>
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
