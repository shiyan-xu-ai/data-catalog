import { useState } from "react";
import { Outlet, Link, createRootRouteWithContext } from "@tanstack/react-router";
import { QueryClient } from "@tanstack/react-query";
import { ChevronRight, Menu } from "lucide-react";
import { joinId } from "@/lib/ids";
import { Toaster } from "@/components/ui/sonner";
import { Button } from "@/components/ui/button";
import { TooltipProvider } from "@/components/ui/tooltip";
import { ThemeToggle } from "@/components/theme-toggle";
import { AppSidebar, useCurrentLocation } from "@/components/app-sidebar";

interface RouterContext {
  queryClient: QueryClient;
}

export const Route = createRootRouteWithContext<RouterContext>()({
  component: RootComponent,
});

/** `Catalog / a / b / table` trail; every namespace prefix links to its page. */
function Breadcrumb() {
  const { segments, isTable } = useCurrentLocation();
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
          <span className="font-medium text-foreground" aria-current="page">Namespaces</span>
        </>
      )}
    </nav>
  );
}

function RootComponent() {
  const [mobileNav, setMobileNav] = useState(false);
  return (
    <TooltipProvider delayDuration={200}>
      <div className="flex h-screen overflow-hidden bg-background">
        <AppSidebar className="hidden md:flex" />
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
            <div className="ml-auto flex items-center gap-1">
              <ThemeToggle />
            </div>
          </header>
          <main className="min-h-0 flex-1 overflow-y-auto">
            <div className="mx-auto w-full max-w-[1600px] px-4 py-6 md:px-8">
              <Outlet />
            </div>
          </main>
        </div>
        <Toaster />
      </div>
    </TooltipProvider>
  );
}
