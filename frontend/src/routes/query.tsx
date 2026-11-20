import { createFileRoute } from "@tanstack/react-router";

// The page itself is rendered by the root layout, which keeps it mounted once visited.
export const Route = createFileRoute("/query")({
  validateSearch: (search: Record<string, unknown>): { table?: string } =>
    typeof search.table === "string" && search.table !== "" ? { table: search.table } : {},
  component: () => null,
});
