import { createFileRoute, useNavigate } from "@tanstack/react-router";
import { useQueryClient } from "@tanstack/react-query";
import { BarChart3, Layers, ListTree, SlidersHorizontal, Table2, TableProperties, Trash2 } from "lucide-react";
import { toast } from "sonner";
import { ApiError, deregisterTable } from "@/lib/api";
import {
  keys,
  useColumnStorage,
  useTableCheap,
  useTableDetail,
  useTableIndices,
  useTableStorage,
  useTableVersions,
} from "@/lib/catalog-queries";
import { formatCount } from "@/lib/format";
import { displayName, joinId, splitId } from "@/lib/ids";
import { ErrorCode } from "@/lib/schemas";
import { storageFormatVersion } from "@/lib/table-metadata";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { ConfirmDialog } from "@/components/confirm-dialog";
import { EmptyState } from "@/components/empty-state";
import { ErrorState } from "@/components/error-state";
import { DetailsSidebar } from "@/components/table/details-sidebar";
import { IndicesTab } from "@/components/table/indices-tab";
import { OverviewTab } from "@/components/table/overview-tab";
import { PropertiesTab } from "@/components/table/properties-tab";
import { SchemaTab } from "@/components/table/schema-tab";
import { VersionsTab } from "@/components/table/versions-tab";

const TABS = ["overview", "properties", "schema", "versions", "indices"] as const;
type Tab = (typeof TABS)[number];

export const Route = createFileRoute("/tables/$tableId")({
  validateSearch: (search: Record<string, unknown>): { tab?: Tab } =>
    TABS.includes(search.tab as Tab) && search.tab !== "overview" ? { tab: search.tab as Tab } : {},
  component: RouteComponent,
});

function RouteComponent() {
  const { tableId } = Route.useParams();
  // Keyed so tab-local state (paging, expansion) resets between tables.
  return <TablePage key={tableId} tableId={tableId} />;
}

function TablePage({ tableId }: { tableId: string }) {
  const segments = splitId(tableId);
  const namespace = segments.slice(0, -1);
  const { tab = "overview" } = Route.useSearch();
  const qc = useQueryClient();
  const navigate = useNavigate();

  const cheap = useTableCheap(segments);
  const detail = useTableDetail(segments);
  const versions = useTableVersions(segments);
  const indices = useTableIndices(segments);
  // Per-column storage and the storage listing read every fragment or object; start them only
  // once the rest of the page settled.
  const settled = !detail.isPending && !versions.isPending && !indices.isPending;
  const columns = useColumnStorage(segments, settled);
  const storage = useTableStorage(segments, settled);

  async function deregister() {
    try {
      await deregisterTable(segments);
    } catch (e) {
      toast.error((e as Error).message);
      throw e;
    }
    toast.success(`Deregistered "${displayName(segments)}"`);
    qc.removeQueries({ queryKey: keys.table(tableId) });
    qc.invalidateQueries({ queryKey: keys.tables(joinId(namespace)) });
    qc.invalidateQueries({ queryKey: keys.allTables });
    navigate(namespace.length ? { to: "/namespaces/$nsId", params: { nsId: joinId(namespace) } } : { to: "/namespaces" });
  }

  if (cheap.error instanceof ApiError && cheap.error.code === ErrorCode.TableNotFound) {
    return (
      <EmptyState icon={Table2} title="Table not found" className="py-16">
        <span className="font-mono">{displayName(segments)}</span> is not registered in the catalog.
      </EmptyState>
    );
  }

  const name = segments[segments.length - 1];
  const format = storageFormatVersion(detail.data?.metadata);
  const tabIcon = {
    overview: BarChart3,
    properties: SlidersHorizontal,
    schema: TableProperties,
    versions: Layers,
    indices: ListTree,
  };
  const counts: Partial<Record<Tab, number | undefined>> = {
    schema: detail.data?.schema?.fields.length,
    versions: versions.data?.length,
    indices: indices.data?.length,
  };

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div className="flex min-w-0 items-start gap-3">
          <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg border bg-muted/50">
            <Table2 className="h-5 w-5 text-muted-foreground" />
          </div>
          <div className="min-w-0 space-y-1">
            <h1 className="truncate font-mono text-xl font-semibold tracking-tight">{name}</h1>
            <div className="flex flex-wrap items-center gap-2 text-sm text-muted-foreground">
              <span className="font-mono">{displayName(namespace)}</span>
              {detail.isPending ? (
                <Skeleton className="h-5 w-24" />
              ) : (
                <>
                  {format && <Badge variant="outline">Lance {format}</Badge>}
                  {detail.data?.version != null && <Badge variant="outline">v{detail.data.version}</Badge>}
                </>
              )}
            </div>
          </div>
        </div>
        {cheap.data && (
          <ConfirmDialog
            trigger={
              <Button variant="outline" className="text-destructive hover:text-destructive">
                <Trash2 /> Deregister
              </Button>
            }
            title="Deregister table"
            description={
              <>
                Remove <span className="font-mono text-foreground">{displayName(segments)}</span> from the
                catalog? The dataset at its location is not modified or deleted.
              </>
            }
            confirmLabel="Deregister"
            destructive
            onConfirm={deregister}
          />
        )}
      </div>

      {cheap.error && <ErrorState message={cheap.error.message} />}

      <div className="grid items-start gap-6 lg:grid-cols-[minmax(0,1fr)_20rem]">
        <Tabs
          value={tab}
          onValueChange={(v) =>
            navigate({ to: ".", search: v === "overview" ? {} : { tab: v as Tab }, replace: true })
          }
          className="min-w-0"
        >
          <TabsList>
            {TABS.map((t) => {
              const Icon = tabIcon[t];
              return (
                <TabsTrigger key={t} value={t} className="capitalize">
                  <Icon /> {t}
                  {counts[t] != null && (
                    <span className="rounded-full bg-muted px-1.5 text-[11px] text-muted-foreground tabular-nums">
                      {formatCount(counts[t])}
                    </span>
                  )}
                </TabsTrigger>
              );
            })}
          </TabsList>
          <TabsContent value="overview">
            <OverviewTab detail={detail} versions={versions} indices={indices} columns={columns} storage={storage} />
          </TabsContent>
          <TabsContent value="properties">
            <PropertiesTab cheap={cheap} detail={detail} />
          </TabsContent>
          <TabsContent value="schema">
            <SchemaTab detail={detail} />
          </TabsContent>
          <TabsContent value="versions">
            <VersionsTab versions={versions} currentVersion={detail.data?.version} />
          </TabsContent>
          <TabsContent value="indices">
            <IndicesTab indices={indices} />
          </TabsContent>
        </Tabs>
        <div className="lg:sticky lg:top-0">
          {cheap.isPending ? (
            <Skeleton className="h-96 w-full" />
          ) : (
            <DetailsSidebar segments={segments} cheap={cheap.data} detail={detail} />
          )}
        </div>
      </div>
    </div>
  );
}
