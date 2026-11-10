import { createFileRoute, useNavigate } from "@tanstack/react-router";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Trash2 } from "lucide-react";
import { toast } from "sonner";
import { deregisterTable, describeTable } from "@/lib/api";
import { formatCount } from "@/lib/format";
import { displayName, joinId, splitId } from "@/lib/ids";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { ConfirmDialog } from "@/components/confirm-dialog";
import { EmptyState } from "@/components/empty-state";
import { ErrorState } from "@/components/error-state";
import { NamespaceBreadcrumb } from "@/components/namespace-breadcrumb";
import { PropertiesList } from "@/components/properties-list";
import { SchemaView } from "@/components/schema-view";

export const Route = createFileRoute("/tables/$tableId")({
  component: TableDetailPage,
});

function MetaRow({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="grid grid-cols-[8rem_1fr] gap-x-4 py-1">
      <dt className="text-xs uppercase tracking-wide text-muted-foreground">{label}</dt>
      <dd className="text-sm wrap-break-word">{children}</dd>
    </div>
  );
}

function TableDetailPage() {
  const { tableId } = Route.useParams();
  const segments = splitId(tableId);
  const namespace = segments.slice(0, -1);
  const qc = useQueryClient();
  const navigate = useNavigate();

  const { data: table, isLoading, error } = useQuery({
    queryKey: ["table", tableId],
    queryFn: () => describeTable(segments),
  });

  async function deregister() {
    try {
      await deregisterTable(segments);
    } catch (e) {
      toast.error((e as Error).message);
      throw e;
    }
    toast.success(`Deregistered "${displayName(segments)}"`);
    qc.removeQueries({ queryKey: ["table", tableId] });
    qc.invalidateQueries({ queryKey: ["tables", joinId(namespace)] });
    navigate(
      namespace.length
        ? { to: "/namespaces/$nsId", params: { nsId: joinId(namespace) } }
        : { to: "/namespaces" },
    );
  }

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-end justify-between gap-4">
        <div className="space-y-1">
          <NamespaceBreadcrumb segments={namespace} />
          <h1 className="font-mono text-2xl">{displayName(segments)}</h1>
        </div>
        {table && (
          <ConfirmDialog
            trigger={
              <Button variant="destructive">
                <Trash2 /> Deregister
              </Button>
            }
            title="Deregister table"
            description={
              <>
                Remove <span className="font-mono">{displayName(segments)}</span> from the catalog?
                The data at its location is not deleted.
              </>
            }
            confirmLabel="Deregister"
            destructive
            onConfirm={deregister}
          />
        )}
      </div>

      {error ? (
        <ErrorState message={(error as Error).message} />
      ) : isLoading ? (
        <div className="py-8 text-center text-sm text-muted-foreground">Loading…</div>
      ) : !table ? (
        <EmptyState>Table not found.</EmptyState>
      ) : (
        <>
          <Card>
            <CardHeader><CardTitle>Overview</CardTitle></CardHeader>
            <CardContent>
              <dl>
                <MetaRow label="Location">
                  <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs break-all">
                    {table.location ?? table.table_uri ?? "—"}
                  </code>
                </MetaRow>
                <MetaRow label="Owner">{table.properties?.owner ?? "—"}</MetaRow>
                <MetaRow label="Version">{table.version ?? "—"}</MetaRow>
                <MetaRow label="Fragments">{formatCount(table.stats?.num_fragments)}</MetaRow>
                <MetaRow label="Deleted rows">{formatCount(table.stats?.num_deleted_rows)}</MetaRow>
              </dl>
            </CardContent>
          </Card>

          <Card>
            <CardHeader><CardTitle>Properties</CardTitle></CardHeader>
            <CardContent>
              <PropertiesList properties={table.properties} />
            </CardContent>
          </Card>

          <Card>
            <CardHeader><CardTitle>Schema</CardTitle></CardHeader>
            <CardContent>
              {table.schema ? (
                <SchemaView schema={table.schema} />
              ) : (
                <EmptyState>Schema unavailable.</EmptyState>
              )}
            </CardContent>
          </Card>
        </>
      )}
    </div>
  );
}
