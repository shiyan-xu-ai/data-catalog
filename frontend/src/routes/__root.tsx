import { Suspense, lazy, useEffect, useState } from "react";
import { Outlet, Link, createRootRouteWithContext, useRouterState, useSearch } from "@tanstack/react-router";
import { QueryClient } from "@tanstack/react-query";
import { ChevronRight, Menu } from "lucide-react";
import { rememberExploreLocation } from "@/lib/explore-location";
import { joinId } from "@/lib/ids";
import { cn } from "@/lib/utils";
import { Toaster } from "@/components/ui/sonner";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { TooltipProvider } from "@/components/ui/tooltip";
import { AppSidebar, useCurrentLocation } from "@/components/app-sidebar";

// The editor (CodeMirror) is the heaviest part of the UI; load it when the Query page first opens.
const QueryPage = lazy(() => import("@/components/query/query-page").then((m) => ({ default: m.QueryPage })));

interface RouterContext {
  queryClient: QueryClient;
}

export const Route = createRootRouteWithContext<RouterContext>()({
  component: RootComponent,
});

function Breadcrumb() {
  const { segments, isTable, isQuery } = useCurrentLocation();
  return (
    <nav aria-label="Breadcrumb" className="flex min-w-0 items-center gap-1 text-sm text-muted-foreground">
      <Link to="/namespaces" className="shrink-0 hover:text-foreground">
        Catalog
      </Link>
      {segments.map((seg, i) => {
        const last = i === segments.length - 1;
        return (
          <span key={i} className="flex min-w-0 items-center gap-1">
            <ChevronRight className="h-3.5 w-3.5 shrink-0 opacity-60" />
            {last ? (
              <span className="truncate font-medium text-foreground" aria-current="page">{seg}</span>
            ) : (
              <Link
                to="/namespaces/$nsId"
                params={{ nsId: joinId(segments.slice(0, i + 1)) }}
                className="truncate hover:text-foreground"
              >
                {seg}
              </Link>
            )}
          </span>
        );
      })}
      {segments.length === 0 && !isTable && (
        <>
          <ChevronRight className="h-3.5 w-3.5 shrink-0 opacity-60" />
          <span className="font-medium text-foreground" aria-current="page">{isQuery ? "Query" : "Namespaces"}</span>
        </>
      )}
    </nav>
  );
}

function RootComponent() {
  const [mobileNav, setMobileNav] = useState(false);
  const { isQuery } = useCurrentLocation();
  const { table } = useSearch({ strict: false });
  const href = useRouterState({ select: (s) => s.location.href });
  useEffect(() => {
    if (href.split(/[?#]/)[0] !== "/query") rememberExploreLocation(href);
  }, [href]);
  // The Query page stays mounted, hidden, once visited, so its tabs, results, and running queries
  // survive a visit to another page.
  const [queryVisited, setQueryVisited] = useState(isQuery);
  if (isQuery && !queryVisited) setQueryVisited(true);
  return (
    <TooltipProvider delayDuration={200}>
      <div className="flex h-screen overflow-hidden bg-background">
        <AppSidebar className="hidden md:flex" collapsible />
        {mobileNav && (
          <div className="fixed inset-0 z-50 flex md:hidden">
            <AppSidebar className="shadow-xl" onNavigate={() => setMobileNav(false)} />
            <button
              type="button"
              aria-label="Close navigation"
              className="flex-1 bg-black/40"
              onClick={() => setMobileNav(false)}
            />
          </div>
        )}
        <div className="flex min-w-0 flex-1 flex-col">
          <header className="flex h-14 shrink-0 items-center gap-3 border-b bg-background/95 px-4 backdrop-blur md:px-6">
            <Button
              variant="ghost"
              size="icon"
              className="md:hidden"
              aria-label="Open navigation"
              onClick={() => setMobileNav(true)}
            >
              <Menu />
            </Button>
            <Breadcrumb />
          </header>
          {/* The Query page fills the main region and scrolls inside its own panes. */}
          <main className={cn("min-h-0 flex-1", isQuery ? "overflow-hidden" : "overflow-y-auto")}>
            <div className={cn("mx-auto w-full max-w-[1600px] px-4 py-6 md:px-8", isQuery && "hidden")}>
              <Outlet />
            </div>
            {queryVisited && (
              <div className={cn("h-full", !isQuery && "hidden")}>
                <Suspense fallback={<Skeleton className="h-full w-full rounded-none" />}>
                  <QueryPage active={isQuery} table={isQuery ? table : undefined} />
                </Suspense>
              </div>
            )}
          </main>
        </div>
        <Toaster />
      </div>
    </TooltipProvider>
  );
}
