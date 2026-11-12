import { Suspense, lazy, useMemo, useState } from "react";
import { createFileRoute, useNavigate } from "@tanstack/react-router";
import {
  BarChart3,
  GitCommitVertical,
  History,
  ListTree,
  SlidersHorizontal,
  Table2,
  TableProperties,
  TerminalSquare,
} from "lucide-react";
import { ApiError } from "@/lib/api";
import {
  useTableCheap,
  useTableDetail,
  useFragmentMeasures,
  useTableIndices,
  useTableStats,
  useTableStorage,
  useTableVersions,
} from "@/lib/catalog-queries";
import { formatCount } from "@/lib/format";
import { displayName, splitId } from "@/lib/ids";
import { ErrorCode } from "@/lib/schemas";
import { Skeleton } from "@/components/ui/skeleton";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { EmptyState } from "@/components/empty-state";
import { LanceIcon } from "@/components/lance-icon";
import { ErrorState } from "@/components/error-state";
import { IndicesTab } from "@/components/table/indices-tab";
import { OverviewTab } from "@/components/table/overview-tab";
import { PropertiesTab } from "@/components/table/properties-tab";
import { SchemaTab } from "@/components/table/schema-tab";
import { TimelineTab } from "@/components/table/timeline-tab";
import { VersionsTab } from "@/components/table/versions-tab";

// The editor (CodeMirror) is the heaviest part of the page; load it when the SQL tab first opens.
const SqlPanel = lazy(() => import("@/components/table/sql-panel").then((m) => ({ default: m.SqlPanel })));

const TABS = ["overview", "properties", "timeline", "versions", "schema", "indices", "sql"] as const;
type Tab = (typeof TABS)[number];
const TAB_LABEL: Partial<Record<Tab, string>> = { sql: "SQL" };

export const Route = createFileRoute("/tables/$tableId")({
  validateSearch: (search: Record<string, unknown>): { tab?: Tab } =>
    TABS.includes(search.tab as Tab) && search.tab !== "overview" ? { tab: search.tab as Tab } : {},
  component: RouteComponent,
});

function RouteComponent() {
  const { tableId } = Route.useParams();
  // Keyed so tab-local state (paging, expansion, the SQL editor) resets between tables.
  return <TablePage key={tableId} tableId={tableId} />;
}

function TablePage({ tableId }: { tableId: string }) {
  const segments = useMemo(() => splitId(tableId), [tableId]);
  const { tab = "overview" } = Route.useSearch();
  const navigate = useNavigate();
  // The SQL tab mounts on first visit and then stays mounted, so its query and results survive
  // switching tabs.
  const [sqlOpened, setSqlOpened] = useState(tab === "sql");
  if (tab === "sql" && !sqlOpened) setSqlOpened(true);
  const cheap = useTableCheap(segments);
  const detail = useTableDetail(segments);
  const versions = useTableVersions(segments);
  const indices = useTableIndices(segments);
  // The storage listing and table stats read every object or fragment; start them only once the
  // rest of the page (and the first version page) settled.
  const settled = !detail.isPending && !versions.isPending && !indices.isPending;
  const storage = useTableStorage(segments, settled);
  const stats = useTableStats(segments, settled);
  const measures = useFragmentMeasures(segments);
  const columnNames = useMemo(() => detail.data?.schema?.fields.map((f) => f.name) ?? [], [detail.data]);

  if (cheap.error instanceof ApiError && cheap.error.code === ErrorCode.TableNotFound) {
    return (
      <EmptyState icon={Table2} title="Table not found" className="py-16">
        <span className="font-mono">{displayName(segments)}</span> is not registered in the catalog.
      </EmptyState>
    );
  }

  const name = segments[segments.length - 1];
  const tabIcon = {
    overview: BarChart3,
    properties: SlidersHorizontal,
    timeline: GitCommitVertical,
    versions: History,
    schema: TableProperties,
    indices: ListTree,
    sql: TerminalSquare,
  };
  const counts: Partial<Record<Tab, number | undefined>> = {
    schema: detail.data?.schema?.fields.length,
    indices: indices.data?.length,
  };

  return (
    <div className="space-y-6">
      <div className="flex min-w-0 flex-wrap items-center gap-3">
        <LanceIcon className="h-9 w-9 shrink-0 rounded-lg" />
        <h1 className="min-w-0 truncate font-mono text-xl font-semibold tracking-tight">{name}</h1>
      </div>

      {cheap.error && <ErrorState message={cheap.error.message} />}

      <Tabs
        value={tab}
        onValueChange={(v) => navigate({ to: ".", search: v === "overview" ? {} : { tab: v as Tab }, replace: true })}
        className="min-w-0 [--tabs-h:2.25rem]"
      >
        <TabsList className="sticky top-0 z-30 h-(--tabs-h) bg-background">
          {TABS.map((t) => {
            const Icon = tabIcon[t];
            return (
              <TabsTrigger key={t} value={t} className="capitalize">
                <Icon /> {TAB_LABEL[t] ?? t}
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
          <OverviewTab cheap={cheap} detail={detail} versions={versions} storage={storage} stats={stats} measures={measures} />
        </TabsContent>
        <TabsContent value="properties">
          <PropertiesTab cheap={cheap} detail={detail} />
        </TabsContent>
        <TabsContent value="timeline">
          <TimelineTab segments={segments} />
        </TabsContent>
        <TabsContent value="versions">
          <VersionsTab segments={segments} versions={versions} currentVersion={detail.data?.version} />
        </TabsContent>
        <TabsContent value="schema">
          <SchemaTab segments={segments} detail={detail} />
        </TabsContent>
        <TabsContent value="indices">
          <IndicesTab indices={indices} />
        </TabsContent>
        {sqlOpened && (
          <TabsContent value="sql" forceMount className="data-[state=inactive]:hidden">
            {/* `overflow-clip` rounds the corners without becoming the toolbar's sticky scroll box. */}
            <div className="overflow-clip rounded-lg border bg-card shadow-xs">
              <Suspense fallback={<Skeleton className="h-96 w-full rounded-none" />}>
                <SqlPanel segments={segments} columns={columnNames} versions={versions} currentVersion={detail.data?.version} />
              </Suspense>
            </div>
          </TabsContent>
        )}
      </Tabs>
    </div>
  );
}
