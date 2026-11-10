import { createFileRoute } from "@tanstack/react-router";
import { NamespaceBrowser } from "@/components/namespace-browser";

export const Route = createFileRoute("/namespaces/")({
  component: () => <NamespaceBrowser segments={[]} />,
});
