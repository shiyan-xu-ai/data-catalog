import { createFileRoute, Link } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { describeNamespace, listTables } from "@/lib/api";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { ErrorState } from "@/components/error-state";
import { EmptyState } from "@/components/empty-state";
import { formatBytes, ttlPolicyString } from "@/lib/format";

export const Route = createFileRoute("/namespaces/$ns")({
  component: NamespacePage,
  parseParams: ({ ns }) => ({ ns: decodeURIComponent(ns) }),
});

function NamespacePage() {
  const { ns } = Route.useParams();
  const segments = ns.split(".");

  const { data: desc, error: descError } = useQuery({
    queryKey: ["namespace", ns],
    queryFn: () => describeNamespace(ns),
  });

  const { data: tables, isLoading } = useQuery({
    queryKey: ["tables"],
    queryFn: listTables,
  });

  const filtered = (tables ?? []).filter((t) =>
    t.namespace.length === segments.length &&
    t.namespace.every((seg, i) => seg === segments[i]),
  );

  return (
    <div className="space-y-6">
      <div className="space-y-1">
        <div className="flex items-center gap-2 text-sm text-muted-foreground">
          <Link to="/namespaces" className="hover:text-foreground">Namespaces</Link>
          <span>/</span>
        </div>
        <h1 className="font-mono text-2xl">{ns}</h1>
      </div>

      {descError ? (
        <ErrorState message={(descError as Error).message} />
      ) : (
        <Card>
          <CardHeader><CardTitle>Namespace</CardTitle></CardHeader>
          <CardContent>
            <dl className="grid grid-cols-[max-content_1fr] gap-x-4">
              <dt className="text-xs uppercase tracking-wide text-muted-foreground">Tables</dt>
              <dd className="text-sm">{desc?.table_count ?? "…"}</dd>
            </dl>
          </CardContent>
        </Card>
      )}

      <div className="space-y-2">
        <h2 className="text-lg font-semibold">Tables in this namespace</h2>
        {isLoading ? (
          <div className="py-8 text-center text-sm text-muted-foreground">Loading…</div>
        ) : filtered.length === 0 ? (
          <EmptyState>No tables in this namespace.</EmptyState>
        ) : (
          <div className="overflow-x-auto rounded-lg border">
            <table className="w-full text-sm">
              <thead className="border-b bg-muted/40 text-left text-xs uppercase tracking-wide text-muted-foreground">
                <tr>
                  <th className="px-3 py-2">Table</th>
                  <th className="px-3 py-2 text-right">Versions</th>
                  <th className="px-3 py-2 text-right">Size</th>
                  <th className="px-3 py-2">TTL</th>
                </tr>
              </thead>
              <tbody>
                {filtered.map((t) => {
                  const total = t.versions.reduce((s, v) => s + v.storage_bytes_total, 0);
                  return (
                    <tr key={t.id} className="border-b last:border-0 hover:bg-muted/40">
                      <td className="px-3 py-2">
                        <Link to="/tables/$tableId" params={{ tableId: t.id }} className="font-medium text-primary hover:underline">
                          {t.id}
                        </Link>
                      </td>
                      <td className="px-3 py-2 text-right tabular-nums">{t.versions.length}</td>
                      <td className="px-3 py-2 text-right tabular-nums text-muted-foreground">{formatBytes(total)}</td>
                      <td className="px-3 py-2">
                        {t.ttl_policy ? <Badge variant="secondary">{ttlPolicyString(t.ttl_policy)}</Badge> : <span className="text-muted-foreground">none</span>}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
      </div>
    </div>
  );
}
