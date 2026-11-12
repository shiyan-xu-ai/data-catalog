import { Suspense, lazy, useMemo } from "react";
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
  type PagedVersions,
  useTableCheap,
  useTableDetail,
  useTableIndices,
  useTableStats,
  useTableStorage,
  useTableVersions,
} from "@/lib/catalog-queries";
import { formatCount } from "@/lib/format";
import { displayName, splitId } from "@/lib/ids";
import { useStoredState } from "@/lib/preferences";
import { ErrorCode } from "@/lib/schemas";
import { storageFormatVersion } from "@/lib/table-metadata";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { BottomDock } from "@/components/bottom-dock";
import { EmptyState } from "@/components/empty-state";
import { ErrorState } from "@/components/error-state";
import { IndicesTab } from "@/components/table/indices-tab";
import { OverviewTab } from "@/components/table/overview-tab";
import { PropertiesTab } from "@/components/table/properties-tab";
import { SchemaTab } from "@/components/table/schema-tab";
import { SqlDockBar } from "@/components/table/sql-dock-bar";
import { TimelineTab } from "@/components/table/timeline-tab";
import { VersionsTab } from "@/components/table/versions-tab";

// The editor (CodeMirror) is the heaviest part of the page; load it after the tabs.
const SqlPanel = lazy(() => import("@/components/table/sql-panel").then((m) => ({ default: m.SqlPanel })));

const TABS = ["overview", "properties", "timeline", "versions", "schema", "indices"] as const;
type Tab = (typeof TABS)[number];

const DOCK_MIN_HEIGHT = 180;
const DOCK_DEFAULT_HEIGHT = 340;
// Leave the page content at least this much height when growing the SQL dock.
const CONTENT_MIN_HEIGHT = 160;

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

/** SQL panel docked under the page content, beside the left nav. It grows by dragging its top
 *  edge and collapses to a slim bar; height and collapsed state persist. */
function SqlDock({
  segments,
  columns,
  versions,
  servedVersion,
}: {
  segments: readonly string[];
  columns: readonly string[];
  versions: PagedVersions;
  servedVersion: number | null | undefined;
}) {
  const [collapsed, setCollapsed] = useStoredState("sql-dock-collapsed", false);
  const [height, setHeight] = useStoredState("sql-dock-height", DOCK_DEFAULT_HEIGHT);

  function startResize(e: React.PointerEvent<HTMLDivElement>) {
    e.preventDefault();
    const handle = e.currentTarget;
    handle.setPointerCapture(e.pointerId);
    const startY = e.clientY;
    const startHeight = height;
    // handle → dock → dock slot → page column (top bar, scrolling content, dock).
    const column = handle.parentElement?.parentElement?.parentElement;
    const max = Math.max(DOCK_MIN_HEIGHT, (column?.clientHeight ?? window.innerHeight) - 56 - CONTENT_MIN_HEIGHT);
    const onMove = (ev: PointerEvent) =>
      setHeight(Math.round(Math.min(max, Math.max(DOCK_MIN_HEIGHT, startHeight + startY - ev.clientY))));
    const onUp = () => {
      handle.removeEventListener("pointermove", onMove);
      handle.removeEventListener("pointerup", onUp);
    };
    handle.addEventListener("pointermove", onMove);
    handle.addEventListener("pointerup", onUp);
  }

  return (
    <>
      {collapsed && (
        <SqlDockBar
          expanded={false}
          onToggle={() => setCollapsed(false)}
          className="border-t bg-card"
          left={
            <>
              <TerminalSquare className="h-4 w-4 shrink-0 text-muted-foreground" />
              <span className="text-sm font-semibold tracking-tight">SQL</span>
              <Badge variant="outline">Read-only</Badge>
              <span className="text-xs text-muted-foreground">Queries against this table</span>
            </>
          }
        />
      )}

      <div hidden={collapsed} className="relative border-t bg-card" style={{ height }}>
        <div
          role="separator"
          aria-orientation="horizontal"
          aria-label="Resize SQL panel"
          title="Drag to resize"
          onPointerDown={startResize}
          className="absolute inset-x-0 -top-1.5 z-10 h-3 cursor-row-resize after:absolute after:top-1 after:left-1/2 after:h-1 after:w-12 after:-translate-x-1/2 after:rounded-full after:bg-border hover:after:bg-muted-foreground/50"
        />
        <Suspense fallback={<Skeleton className="h-full w-full rounded-none" />}>
          <SqlPanel
            segments={segments}
            columns={columns}
            versions={versions}
            servedVersion={servedVersion}
            onCollapse={() => setCollapsed(true)}
          />
        </Suspense>
      </div>
    </>
  );
}

function TablePage({ tableId }: { tableId: string }) {
  const segments = useMemo(() => splitId(tableId), [tableId]);
  const { tab = "overview" } = Route.useSearch();
  const navigate = useNavigate();

  const cheap = useTableCheap(segments);
  const detail = useTableDetail(segments);
  const versions = useTableVersions(segments);
  const indices = useTableIndices(segments);
  // The storage listing and table stats read every object or fragment; start them only once the
  // rest of the page (and the first version page) settled.
  const settled = !detail.isPending && !versions.isPending && !indices.isPending;
  const storage = useTableStorage(segments, settled);
  const stats = useTableStats(segments, settled);
  const columnNames = useMemo(() => detail.data?.schema?.fields.map((f) => f.name) ?? [], [detail.data]);

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
    timeline: GitCommitVertical,
    versions: History,
    schema: TableProperties,
    indices: ListTree,
  };
  const counts: Partial<Record<Tab, number | undefined>> = {
    schema: detail.data?.schema?.fields.length,
    indices: indices.data?.length,
  };

  return (
    <>
      <div className="space-y-6">
        <div className="flex min-w-0 flex-wrap items-center gap-3">
          <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border bg-muted/50">
            <Table2 className="h-5 w-5 text-muted-foreground" />
          </div>
          <h1 className="min-w-0 truncate font-mono text-xl font-semibold tracking-tight">{name}</h1>
          {detail.isPending ? (
            <Skeleton className="h-5 w-24" />
          ) : (
            <>
              {format && <Badge variant="outline">Lance {format}</Badge>}
              {detail.data?.version != null && <Badge variant="outline">v{detail.data.version}</Badge>}
            </>
          )}
        </div>

        {cheap.error && <ErrorState message={cheap.error.message} />}

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
            <OverviewTab detail={detail} indices={indices} storage={storage} stats={stats} />
          </TabsContent>
          <TabsContent value="properties">
            <PropertiesTab cheap={cheap} detail={detail} />
          </TabsContent>
          <TabsContent value="timeline">
            <TimelineTab segments={segments} />
          </TabsContent>
          <TabsContent value="versions">
            <VersionsTab segments={segments} versions={versions} servedVersion={detail.data?.version} />
          </TabsContent>
          <TabsContent value="schema">
            <SchemaTab segments={segments} detail={detail} />
          </TabsContent>
          <TabsContent value="indices">
            <IndicesTab indices={indices} />
          </TabsContent>
        </Tabs>
      </div>

      <BottomDock>
        <SqlDock
          segments={segments}
          columns={columnNames}
          versions={versions}
          servedVersion={detail.data?.version}
        />
      </BottomDock>
    </>
  );
}
