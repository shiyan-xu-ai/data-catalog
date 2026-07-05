import { createFileRoute, Link } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { listNamespaces } from "@/lib/api";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { ErrorState } from "@/components/error-state";
import { EmptyState } from "@/components/empty-state";
import { nsToString } from "@/lib/format";

export const Route = createFileRoute("/namespaces")({
  component: NamespacesPage,
});

function NamespacesPage() {
  const { data, isLoading, error } = useQuery({
    queryKey: ["namespaces"],
    queryFn: listNamespaces,
  });

  const namespaces = data?.namespaces ?? [];

  return (
    <div className="space-y-4">
      <h1 className="text-2xl font-semibold">Namespaces</h1>
      {error ? (
        <ErrorState message={(error as Error).message} />
      ) : isLoading ? (
        <div className="py-8 text-center text-sm text-muted-foreground">Loading…</div>
      ) : namespaces.length === 0 ? (
        <EmptyState>No namespaces.</EmptyState>
      ) : (
        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
          {namespaces
            .slice()
            .sort((a, b) => nsToString(a).localeCompare(nsToString(b)))
            .map((ns) => {
              const id = nsToString(ns);
              return (
                <Link key={id} to="/namespaces/$ns" params={{ ns: id }} className="block">
                  <Card className="transition-colors hover:border-primary/50 hover:bg-accent/40">
                    <CardHeader className="pb-2">
                      <CardTitle className="font-mono text-sm">{id}</CardTitle>
                    </CardHeader>
                    <CardContent className="pt-0">
                      <p className="text-xs text-muted-foreground">{ns.length} segment{ns.length !== 1 ? "s" : ""}</p>
                    </CardContent>
                  </Card>
                </Link>
              );
            })}
        </div>
      )}
    </div>
  );
}
