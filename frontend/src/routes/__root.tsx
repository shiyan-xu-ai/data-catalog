import { Outlet, Link, createRootRouteWithContext } from "@tanstack/react-router";
import { QueryClient } from "@tanstack/react-query";
import { Toaster } from "@/components/ui/sonner";
import { TooltipProvider } from "@/components/ui/tooltip";

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
        <header className="border-b">
          <div className="container flex h-14 items-center gap-6">
            <Link to="/tables" className="flex items-center gap-2 font-semibold">
              <img src="/logo-light.png" alt="Applied Intuition" className="h-6 w-auto dark:hidden" />
              <img src="/logo-dark.png" alt="" aria-hidden="true" className="hidden h-6 w-auto dark:block" />
              <span>Applied Data Catalog</span>
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
          </div>
        </header>
        <main className="container py-6">
          <Outlet />
        </main>
        <Toaster />
      </div>
    </TooltipProvider>
  );
}
