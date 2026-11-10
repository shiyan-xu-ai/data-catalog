import { createFileRoute } from "@tanstack/react-router";
import { NamespacePage } from "@/components/namespace-page";
import { splitId } from "@/lib/ids";

export const Route = createFileRoute("/namespaces/$nsId")({
  component: RouteComponent,
});

function RouteComponent() {
  const { nsId } = Route.useParams();
  // Keyed so dialog/query state resets when navigating between namespaces.
  return <NamespacePage key={nsId} segments={splitId(nsId)} />;
}
