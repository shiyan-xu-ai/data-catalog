import { createFileRoute } from "@tanstack/react-router";
import { NamespaceBrowser } from "@/components/namespace-browser";
import { splitId } from "@/lib/ids";

export const Route = createFileRoute("/namespaces/$nsId")({
  component: NamespacePage,
});

function NamespacePage() {
  const { nsId } = Route.useParams();
  // Keyed so dialog/query state resets when navigating between namespaces.
  return <NamespaceBrowser key={nsId} segments={splitId(nsId)} />;
}
