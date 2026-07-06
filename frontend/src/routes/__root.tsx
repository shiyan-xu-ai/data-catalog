import { Outlet, Link, createRootRouteWithContext } from "@tanstack/react-router";
import { QueryClient } from "@tanstack/react-query";
import { Toaster } from "@/components/ui/sonner";
import { TooltipProvider } from "@/components/ui/tooltip";
import { ThemeToggle } from "@/components/theme-toggle";
import { TimezoneToggle } from "@/components/timezone-toggle";

interface RouterContext {
  queryClient: QueryClient;
}

export const Route = createRootRouteWithContext<RouterContext>()({
  component: RootComponent,
});

function RootComponent() {
  return (
    <TooltipProvider delayDuration={200}>
      <div className="min-h-screen bg-background">
        <header className="sticky top-0 z-40 border-b bg-background/95 backdrop-blur supports-[backdrop-filter]:bg-background/60">
          <div className="container flex h-14 items-center gap-4 px-4 sm:gap-6">
            <Link to="/tables" className="flex items-center gap-2 font-semibold">
              <img src="/logo-light.png" alt="Applied Intuition" className="h-6 w-auto dark:hidden" />
              <img src="/logo-dark.png" alt="" aria-hidden="true" className="hidden h-6 w-auto dark:block" />
              <span className="hidden sm:inline">Applied Data Catalog</span>
            </Link>
            <nav className="flex items-center gap-4 text-sm">
              <Link
                to="/tables"
                className="text-muted-foreground transition-colors hover:text-foreground [&.active]:text-foreground [&.active]:font-medium"
              >
                Tables
              </Link>
              <Link
                to="/namespaces"
                className="text-muted-foreground transition-colors hover:text-foreground [&.active]:text-foreground [&.active]:font-medium"
              >
                Namespaces
              </Link>
            </nav>
            <div className="ml-auto flex items-center gap-1">
              <TimezoneToggle />
              <ThemeToggle />
            </div>
          </div>
        </header>
        <main className="container px-4 py-6">
          <Outlet />
        </main>
        <Toaster />
      </div>
    </TooltipProvider>
  );
}
