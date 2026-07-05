import { createFileRoute, Link } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { describeTable, ttlAudit } from "@/lib/api";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { ErrorState } from "@/components/error-state";
import { EmptyState } from "@/components/empty-state";
import { TtlPanel } from "@/components/ttl-panel";
import { VersionsTable } from "@/components/versions-table";
import { formatBytes, formatRelative, formatTime, nsToString, ttlPolicyString } from "@/lib/format";

export const Route = createFileRoute("/tables/$tableId/")({
  component: TableDetailPage,
});

function MetaRow({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="grid grid-cols-[max-content_1fr] gap-x-4 gap-y-1 py-1">
      <dt className="text-xs uppercase tracking-wide text-muted-foreground">{label}</dt>
      <dd className="text-sm break-words">{children}</dd>
    </div>
  );
}

function TableDetailPage() {
  const { tableId } = Route.useParams();
  const { data: table, isLoading, error } = useQuery({
    queryKey: ["table", tableId],
    queryFn: () => describeTable(tableId),
  });
  const { data: audit } = useQuery({
    queryKey: ["ttl-audit", tableId],
    queryFn: () => ttlAudit(tableId),
    enabled: !!table,
  });

  if (error) return <ErrorState message={(error as Error).message} />;
  if (isLoading) return <div className="py-8 text-center text-sm text-muted-foreground">Loading…</div>;
  if (!table) return <EmptyState>Table not found.</EmptyState>;

  const latest = table.versions[0];
  const totalBytes = table.versions.reduce((s, v) => s + v.storage_bytes_total, 0);

  return (
    <div className="space-y-6">
      <div className="space-y-1">
        <div className="flex items-center gap-2 text-sm text-muted-foreground">
          <Link to="/tables" className="hover:text-foreground">Tables</Link>
          <span>/</span>
        </div>
        <h1 className="text-2xl font-semibold">{table.id}</h1>
        <p className="text-sm text-muted-foreground">{table.name}</p>
      </div>

      <Card>
        <CardHeader><CardTitle>Overview</CardTitle></CardHeader>
        <CardContent>
          <dl>
            <MetaRow label="Namespace">
              {table.namespace.length ? (
                <Link to="/namespaces/$ns" params={{ ns: nsToString(table.namespace) }} className="text-primary hover:underline">
                  {nsToString(table.namespace)}
                </Link>
              ) : "—"}
            </MetaRow>
            <MetaRow label="Root">{<code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs">{table.root_location}</code>}</MetaRow>
            <MetaRow label="Owner">{table.owner ?? "—"}</MetaRow>
            <MetaRow label="TTL policy">{table.ttl_policy ? <Badge variant="secondary">{ttlPolicyString(table.ttl_policy)}</Badge> : "none"}</MetaRow>
            <MetaRow label="Versions">{table.versions.length}</MetaRow>
            <MetaRow label="Total size">{formatBytes(totalBytes)}</MetaRow>
            <MetaRow label="Last swept">{formatTime(table.last_swept)} <span className="text-muted-foreground">({formatRelative(table.last_swept)})</span></MetaRow>
          </dl>
        </CardContent>
      </Card>

      <TtlPanel tableId={table.id} policy={table.ttl_policy} audit={audit ?? []} />

      <Card>
        <CardHeader>
          <CardTitle>Versions</CardTitle>
        </CardHeader>
        <CardContent>
          {table.versions.length === 0 ? (
            <EmptyState>No versions swept yet.</EmptyState>
          ) : (
            <VersionsTable tableId={table.id} versions={table.versions} />
          )}
        </CardContent>
      </Card>

      {latest && latest.aux.length > 0 && (
        <Card>
          <CardHeader><CardTitle>Latest aux</CardTitle></CardHeader>
          <CardContent>
            <div className="overflow-x-auto">
              <table className="w-full text-sm">
                <thead className="border-b bg-muted/40 text-left text-xs uppercase tracking-wide text-muted-foreground">
                  <tr>
                    <th className="px-3 py-2">Name</th>
                    <th className="px-3 py-2">Role</th>
                    <th className="px-3 py-2">Format</th>
                    <th className="px-3 py-2 text-right">Size</th>
                  </tr>
                </thead>
                <tbody>
                  {latest.aux.map((a) => (
                    <tr key={a.name} className="border-b last:border-0">
                      <td className="px-3 py-2 font-mono text-xs">{a.name}</td>
                      <td className="px-3 py-2 text-muted-foreground">{a.role || a.name}</td>
                      <td className="px-3 py-2"><Badge variant="outline">{a.format}</Badge></td>
                      <td className="px-3 py-2 text-right tabular-nums">{formatBytes(a.storage_bytes)}</td>
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
