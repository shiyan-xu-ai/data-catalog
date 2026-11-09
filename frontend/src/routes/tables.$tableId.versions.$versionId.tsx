import { createFileRoute, Link } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { getVersion } from "@/lib/api";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { ErrorState } from "@/components/error-state";
import { EmptyState } from "@/components/empty-state";
import { ShapeBadge } from "@/components/shape-badge";
import { AuxTable } from "@/components/aux-table";
import { SchemaView } from "@/components/schema-view";
import { formatBytes, formatCount, formatTime } from "@/lib/format";
import { useTz } from "@/lib/timezone";

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
  const { mode: tz } = useTz();

  const { data: version, isLoading, error } = useQuery({
    queryKey: ["version", tableId, versionId],
    queryFn: () => getVersion(tableId, versionId),
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
        </div>
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
              <Stat label="Timestamp" value={formatTime(version.timestamp, tz)} />
              <Stat label="Rows" value={formatCount(version.row_count)} />
              <Stat label="Fragments" value={formatCount(version.num_fragments)} />
              <Stat label="Indices" value={formatCount(version.num_indices)} />
              <Stat label="Synced" value={formatTime(version.synced_at, tz)} />
              <Stat label="Snapshot" value={<code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs">{version.snapshot_path}</code>} />
            </dl>
          </CardContent>
        </Card>
      </div>

      {version.schema_json && (
        <Card>
          <CardHeader><CardTitle>Schema</CardTitle></CardHeader>
          <CardContent>
            <SchemaView schemaJson={version.schema_json} />
          </CardContent>
        </Card>
      )}

      {version.aux.length > 0 && (
        <Card>
          <CardHeader><CardTitle>Auxiliary data</CardTitle></CardHeader>
          <CardContent>
            <AuxTable tableId={tableId} versionId={version.version_id} aux={version.aux} />
          </CardContent>
        </Card>
      )}
    </div>
  );
}
