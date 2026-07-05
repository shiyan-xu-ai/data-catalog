import { useState } from "react";
import { createFileRoute, Link } from "@tanstack/react-router";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { Shield, ShieldOff } from "lucide-react";
import { getVersion, protectVersion } from "@/lib/api";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { ErrorState } from "@/components/error-state";
import { EmptyState } from "@/components/empty-state";
import { ShapeBadge } from "@/components/shape-badge";
import { CodeBlock, prettyJson } from "@/components/code-block";
import { formatBytes, formatTime } from "@/lib/format";

export const Route = createFileRoute("/tables/$tableId/versions/$versionId")({
  component: VersionDetailPage,
});

function Byte({ label, value }: { label: string; value: number }) {
  return (
    <div className="grid grid-cols-[max-content_1fr] gap-x-3 py-1">
      <dt className="text-xs uppercase tracking-wide text-muted-foreground">{label}</dt>
      <dd className="text-sm tabular-nums">{formatBytes(value)}</dd>
    </div>
  );
}

function Stat({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div className="grid grid-cols-[max-content_1fr] gap-x-3 py-1">
      <dt className="text-xs uppercase tracking-wide text-muted-foreground">{label}</dt>
      <dd className="text-sm">{value}</dd>
    </div>
  );
}

function VersionDetailPage() {
  const { tableId, versionId } = Route.useParams();
  const qc = useQueryClient();
  const [protecting, setProtecting] = useState(false);

  const { data: version, isLoading, error } = useQuery({
    queryKey: ["version", tableId, versionId],
    queryFn: () => getVersion(tableId, versionId),
  });

  const protectMut = useMutation({
    mutationFn: (protect: boolean) => protectVersion(tableId, versionId, protect),
    onMutate: () => setProtecting(true),
    onSettled: () => setProtecting(false),
    onSuccess: (v) => {
      toast.success(v.protected ? "Protected" : "Unprotected");
      qc.invalidateQueries({ queryKey: ["version", tableId, versionId] });
      qc.invalidateQueries({ queryKey: ["table", tableId] });
    },
    onError: (e: unknown) => toast.error((e as Error).message),
  });

  if (error) return <ErrorState message={(error as Error).message} />;
  if (isLoading) return <div className="py-8 text-center text-sm text-muted-foreground">Loading…</div>;
  if (!version) return <EmptyState>Version not found.</EmptyState>;

  return (
    <div className="space-y-6">
      <div className="space-y-1">
        <div className="flex items-center gap-2 text-sm text-muted-foreground">
          <Link to="/tables" className="hover:text-foreground">Tables</Link>
          <span>/</span>
          <Link to="/tables/$tableId" params={{ tableId }} className="hover:text-foreground">{tableId}</Link>
          <span>/</span>
          <span className="text-foreground">versions</span>
        </div>
        <div className="flex items-center gap-3">
          <h1 className="font-mono text-xl">{version.version_id}</h1>
          <ShapeBadge shape={version.shape} />
          {version.partial && <Badge variant="warning">partial</Badge>}
          {version.protected && <Badge variant="success">protected</Badge>}
        </div>
      </div>

      <div className="flex gap-2">
        <Button
          size="sm"
          variant={version.protected ? "default" : "outline"}
          disabled={protecting}
          onClick={() => protectMut.mutate(!version.protected)}
        >
          {version.protected ? <ShieldOff className="h-4 w-4" /> : <Shield className="h-4 w-4" />}
          {version.protected ? "Unprotect" : "Protect"}
        </Button>
      </div>

      <div className="grid gap-6 lg:grid-cols-2">
        <Card>
          <CardHeader><CardTitle>Storage</CardTitle></CardHeader>
          <CardContent>
            <dl>
              <Byte label="Total" value={version.storage_bytes_total} />
              <Byte label="Lance core" value={version.lance_core_bytes} />
              <Byte label="Sidecar" value={version.sidecar_bytes} />
              <Byte label="Segments" value={version.segments_bytes} />
              <Byte label="Other aux" value={version.other_aux_bytes} />
            </dl>
          </CardContent>
        </Card>

        <Card>
          <CardHeader><CardTitle>Stats</CardTitle></CardHeader>
          <CardContent>
            <dl>
              <Stat label="Timestamp" value={formatTime(version.timestamp)} />
              <Stat label="Rows" value={version.row_count != null ? version.row_count.toLocaleString() : "—"} />
              <Stat label="Fragments" value={version.num_fragments != null ? version.num_fragments.toLocaleString() : "—"} />
              <Stat label="Indices" value={version.num_indices != null ? version.num_indices.toLocaleString() : "—"} />
              <Stat label="Swept" value={formatTime(version.swept_at)} />
              <Stat label="Snapshot" value={<code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs">{version.snapshot_path}</code>} />
            </dl>
          </CardContent>
        </Card>
      </div>

      {version.schema_json && (
        <Card>
          <CardHeader><CardTitle>Schema</CardTitle></CardHeader>
          <CardContent>
            <CodeBlock>{prettyJson(version.schema_json)}</CodeBlock>
          </CardContent>
        </Card>
      )}

      {version.aux.length > 0 && (
        <Card>
          <CardHeader><CardTitle>Aux directories</CardTitle></CardHeader>
          <CardContent>
            <div className="overflow-x-auto">
              <table className="w-full text-sm">
                <thead className="border-b bg-muted/40 text-left text-xs uppercase tracking-wide text-muted-foreground">
                  <tr>
                    <th className="px-3 py-2">Name</th>
                    <th className="px-3 py-2">Role</th>
                    <th className="px-3 py-2">Format</th>
                    <th className="px-3 py-2 text-right">Size</th>
                    <th className="px-3 py-2">Path</th>
                  </tr>
                </thead>
                <tbody>
                  {version.aux.map((a) => (
                    <tr key={a.name} className="border-b last:border-0">
                      <td className="px-3 py-2 font-mono text-xs">{a.name}</td>
                      <td className="px-3 py-2 text-muted-foreground">{a.role || a.name}</td>
                      <td className="px-3 py-2"><Badge variant="outline">{a.format}</Badge></td>
                      <td className="px-3 py-2 text-right tabular-nums">{formatBytes(a.storage_bytes)}</td>
                      <td className="px-3 py-2 font-mono text-xs text-muted-foreground">{a.path}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </CardContent>
        </Card>
      )}
    </div>
  );
}
