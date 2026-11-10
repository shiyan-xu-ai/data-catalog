import { Suspense, lazy, useMemo } from "react";
import { createFileRoute, useNavigate } from "@tanstack/react-router";
import { BarChart3, Layers, ListTree, PanelRightOpen, SlidersHorizontal, Table2, TableProperties } from "lucide-react";
import { ApiError } from "@/lib/api";
import {
  useColumnStorage,
  useTableCheap,
  useTableDetail,
  useTableIndices,
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
import { EmptyState } from "@/components/empty-state";
import { ErrorState } from "@/components/error-state";
import { IndicesTab } from "@/components/table/indices-tab";
import { OverviewTab } from "@/components/table/overview-tab";
import { PropertiesTab } from "@/components/table/properties-tab";
import { SchemaTab } from "@/components/table/schema-tab";
import { VersionsTab } from "@/components/table/versions-tab";

// The editor (CodeMirror) is the heaviest part of the page; load it after the tabs.
const SqlPanel = lazy(() => import("@/components/table/sql-panel").then((m) => ({ default: m.SqlPanel })));

const TABS = ["overview", "properties", "schema", "versions", "indices"] as const;
type Tab = (typeof TABS)[number];

const PANEL_MIN_WIDTH = 320;
const PANEL_DEFAULT_WIDTH = 480;
// Leave the tabs at least this much room when widening the SQL panel.
const MAIN_MIN_WIDTH = 420;

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

/** Right-hand SQL panel: collapsible to a narrow tab and widened by dragging its left edge.
 *  Both choices persist. Below `lg` it stacks under the tabs at full width. */
function SqlDock({ segments, columns }: { segments: readonly string[]; columns: readonly string[] }) {
  const [collapsed, setCollapsed] = useStoredState("sql-panel-collapsed", false);
  const [width, setWidth] = useStoredState("sql-panel-width", PANEL_DEFAULT_WIDTH);

  function startResize(e: React.PointerEvent<HTMLDivElement>) {
    e.preventDefault();
    const handle = e.currentTarget;
    handle.setPointerCapture(e.pointerId);
    const startX = e.clientX;
    // The page row (tabs + gap + panel) bounds how wide the panel may grow.
    const row = handle.parentElement?.parentElement;
    const max = Math.max(PANEL_MIN_WIDTH, (row?.clientWidth ?? window.innerWidth) - MAIN_MIN_WIDTH - 24);
    const startWidth = width;
    const onMove = (ev: PointerEvent) =>
      setWidth(Math.round(Math.min(max, Math.max(PANEL_MIN_WIDTH, startWidth + startX - ev.clientX))));
    const onUp = () => {
      handle.removeEventListener("pointermove", onMove);
      handle.removeEventListener("pointerup", onUp);
    };
    handle.addEventListener("pointermove", onMove);
    handle.addEventListener("pointerup", onUp);
  }

  if (collapsed) {
    return (
      <button
        type="button"
        onClick={() => setCollapsed(false)}
        aria-label="Open SQL panel"
        className="flex shrink-0 items-center gap-2 self-start rounded-lg border bg-card px-3 py-2 text-xs font-medium text-muted-foreground shadow-xs hover:text-foreground lg:sticky lg:top-6 lg:flex-col lg:px-1.5 lg:py-3"
      >
        <PanelRightOpen className="h-4 w-4" />
        <span className="lg:[writing-mode:vertical-rl]">SQL</span>
      </button>
    );
  }

  return (
    <div
      className="relative h-[36rem] w-full shrink-0 lg:sticky lg:top-6 lg:h-[calc(100vh-3.5rem-3rem)] lg:w-[var(--sql-panel-width)]"
      style={{ "--sql-panel-width": `${width}px` } as React.CSSProperties}
    >
      <div
        role="separator"
        aria-orientation="vertical"
        aria-label="Resize SQL panel"
        title="Drag to resize"
        onPointerDown={startResize}
        className="absolute inset-y-0 -left-3 z-10 hidden w-3 cursor-col-resize lg:block after:absolute after:inset-y-4 after:left-1 after:w-0.5 after:rounded-full after:bg-transparent hover:after:bg-border"
      />
      <Suspense fallback={<Skeleton className="h-full w-full" />}>
        <SqlPanel segments={segments} columns={columns} onCollapse={() => setCollapsed(true)} />
      </Suspense>
    </div>
  );
}

function TablePage({ tableId }: { tableId: string }) {
  const segments = useMemo(() => splitId(tableId), [tableId]);
  const namespace = segments.slice(0, -1);
  const { tab = "overview" } = Route.useSearch();
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
    <div className="flex flex-col gap-6 lg:flex-row lg:items-start">
      <div className="min-w-0 flex-1 space-y-6">
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

        {cheap.error && <ErrorState message={cheap.error.message} />}

        <Tabs
          value={tab}
          onValueChange={(v) =>
            navigate({ to: ".", search: v === "overview" ? {} : { tab: v as Tab }, replace: true })
          }
          className="min-w-0"
        >
          <TabsList className="overflow-x-auto">
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
            <PropertiesTab segments={segments} cheap={cheap} detail={detail} />
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
      </div>

      <SqlDock segments={segments} columns={columnNames} />
    </div>
  );
}
