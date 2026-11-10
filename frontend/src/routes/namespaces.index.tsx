import { createFileRoute } from "@tanstack/react-router";
import { NamespacePage } from "@/components/namespace-page";

export const Route = createFileRoute("/namespaces/")({
  component: () => <NamespacePage segments={[]} />,
});
