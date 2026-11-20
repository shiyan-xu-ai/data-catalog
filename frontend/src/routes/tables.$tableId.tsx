import { useEffect, useMemo, useRef, useState } from "react";
import { Link, createFileRoute, useNavigate } from "@tanstack/react-router";
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
import { ActiveContext } from "@/lib/active";
import { ApiError } from "@/lib/api";
import {
  useTableCheap,
  useTableDetail,
  useFragmentMeasures,
  useTableIndices,
  useTableRowCount,
  useTableStats,
  useTableStorage,
  useTableVersions,
} from "@/lib/catalog-queries";
import { formatCount } from "@/lib/format";
import { displayName, splitId } from "@/lib/ids";
import { ErrorCode } from "@/lib/schemas";
import { Button } from "@/components/ui/button";
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

const TABS = ["overview", "properties", "timeline", "versions", "schema", "indices"] as const;
type Tab = (typeof TABS)[number];

export const Route = createFileRoute("/tables/$tableId")({
  // An unset `tab` is explicit so a stale value (a removed tab) does not leak through from the
  // unvalidated parent search.
  validateSearch: (search: Record<string, unknown>): { tab?: Tab } => ({
    tab: TABS.includes(search.tab as Tab) && search.tab !== "overview" ? (search.tab as Tab) : undefined,
  }),
  component: RouteComponent,
});

/** Visited table pages kept mounted; the least recently visited beyond this are dropped. */
const MAX_KEPT_TABLES = 8;

function RouteComponent() {
  const { tableId } = Route.useParams();
  // Pages stay mounted, hidden, once visited, so each table keeps its tab state (selected rows,
  // expansion, paging) while another table is on screen. `order` holds
  // recency; `ids` keeps insertion order so React never moves a page's DOM, which would reset
  // its scroll positions.
  const [kept, setKept] = useState({ ids: [tableId], order: [tableId] });
  if (kept.order[kept.order.length - 1] !== tableId) {
    const order = [...kept.order.filter((id) => id !== tableId), tableId];
    const dropped = order.length > MAX_KEPT_TABLES ? order.shift() : undefined;
    const ids = kept.ids.filter((id) => id !== dropped);
    setKept({ ids: ids.includes(tableId) ? ids : [...ids, tableId], order });
  }
  return (
    <>
      {kept.ids.map((id) => (
        <div key={id} hidden={id !== tableId}>
          <TablePage tableId={id} active={id === tableId} />
        </div>
      ))}
    </>
  );
}

function TablePage({ tableId, active }: { tableId: string; active: boolean }) {
  const segments = useMemo(() => splitId(tableId), [tableId]);
  const urlTab = Route.useSearch().tab ?? "overview";
  const navigate = useNavigate();
  // The shown tab is local so a hidden page keeps its own while the URL follows the table on
  // screen. Returning to a table through a link without `?tab=` restores its tab into the URL;
  // otherwise the URL (a tab click, back, forward) wins.
  const [tab, setTab] = useState<Tab>(urlTab);
  const wasActive = useRef(active);
  useEffect(() => {
    const activated = active && !wasActive.current;
    wasActive.current = active;
    if (!active) return;
    if (activated && urlTab === "overview") {
      if (tab !== "overview") navigate({ to: ".", search: { tab }, replace: true });
    } else if (urlTab !== tab) {
      setTab(urlTab);
    }
    // `tab` is read only when the URL or activity changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, urlTab]);
  // A tab mounts on first visit and then stays mounted, so its state survives switching tabs.
  const [opened, setOpened] = useState<ReadonlySet<Tab>>(() => new Set([tab]));
  if (!opened.has(tab)) setOpened(new Set([...opened, tab]));
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
  const rowCount = useTableRowCount(segments);

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
  };
  const counts: Partial<Record<Tab, number | undefined>> = {
    schema: detail.data?.schema?.fields.length,
    indices: indices.data?.length,
  };
  const select = (v: string) => {
    setTab(v as Tab);
    navigate({ to: ".", search: v === "overview" ? {} : { tab: v as Tab }, replace: true });
  };
  const content: Record<Tab, React.ReactNode> = {
    overview: (
      <OverviewTab
        segments={segments}
        cheap={cheap}
        detail={detail}
        versions={versions}
        storage={storage}
        stats={stats}
        rowCount={rowCount}
        measures={measures}
      />
    ),
    properties: <PropertiesTab cheap={cheap} detail={detail} />,
    timeline: <TimelineTab segments={segments} />,
    versions: <VersionsTab segments={segments} versions={versions} currentVersion={detail.data?.version} />,
    schema: <SchemaTab segments={segments} detail={detail} />,
    indices: <IndicesTab indices={indices} />,
  };

  return (
    <div className="space-y-6">
      <div className="flex min-w-0 flex-wrap items-center gap-3">
        <LanceIcon className="h-9 w-9 shrink-0 rounded-lg" />
        <h1 className="min-w-0 truncate font-mono text-xl font-semibold tracking-tight">{name}</h1>
        <Button asChild variant="outline" size="sm" className="ml-auto">
          <Link to="/query" search={{ table: tableId }}>
            <TerminalSquare />
            Query this table
          </Link>
        </Button>
      </div>

      {cheap.error && <ErrorState message={cheap.error.message} />}

      <Tabs value={tab} onValueChange={select} className="min-w-0 [--tabs-h:2.25rem]">
        <TabsList className="sticky top-0 z-30 h-(--tabs-h) bg-background">
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
        {TABS.filter((t) => opened.has(t)).map((t) => (
          <TabsContent key={t} value={t} forceMount className="data-[state=inactive]:hidden">
            <ActiveContext.Provider value={active && tab === t}>{content[t]}</ActiveContext.Provider>
          </TabsContent>
        ))}
      </Tabs>
    </div>
  );
}
