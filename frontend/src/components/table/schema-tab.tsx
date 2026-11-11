import { useMemo, useState } from "react";
import type { UseQueryResult } from "@tanstack/react-query";
import { Braces, ChevronRight, ListTree, TableProperties } from "lucide-react";
import { expandablePaths, flattenSchema } from "@/lib/schema-tree";
import type { DescribeTableResponse, JsonArrowSchema } from "@/lib/schemas";
import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { CodeBlock } from "@/components/code-block";
import { EmptyState } from "@/components/empty-state";
import { ErrorState } from "@/components/error-state";
import { Identifier } from "@/components/identifier";
import { PropertiesList } from "@/components/properties-list";
import { Section } from "@/components/section";

export function SchemaTab({ detail }: { detail: UseQueryResult<DescribeTableResponse> }) {
  if (detail.error) return <ErrorState message={`Schema unavailable: ${detail.error.message}`} />;
  if (detail.isPending) {
    return (
      <div className="space-y-2">
        {Array.from({ length: 6 }, (_, i) => (
          <Skeleton key={i} className="h-8 w-full" />
        ))}
      </div>
    );
  }
  const schema = detail.data.schema;
  if (!schema) return <EmptyState icon={ListTree} title="No schema">The describe response has no schema.</EmptyState>;
  return <SchemaView schema={schema} />;
}

function SchemaView({ schema }: { schema: JsonArrowSchema }) {
  const [mode, setMode] = useState<"tree" | "raw">("tree");
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set());
  const allPaths = useMemo(() => expandablePaths(schema.fields), [schema]);
  const rows = useMemo(() => flattenSchema(schema.fields, expanded), [schema, expanded]);
  const schemaMeta = Object.entries(schema.metadata ?? {}).sort(([a], [b]) => a.localeCompare(b));

  function toggle(path: string) {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (!next.delete(path)) next.add(path);
      return next;
    });
  }

  return (
    <div className="space-y-6">
      <Section
        title="Fields"
        contentClassName={mode === "tree" ? "px-0 pb-0" : undefined}
        actions={
          <>
            {mode === "tree" && allPaths.length > 0 && (
              <Button
                size="sm"
                variant="ghost"
                onClick={() => setExpanded(expanded.size === allPaths.length ? new Set() : new Set(allPaths))}
              >
                {expanded.size === allPaths.length ? "Collapse all" : "Expand all"}
              </Button>
            )}
            <div className="inline-flex rounded-md border p-0.5">
              <Button size="sm" className="h-7" variant={mode === "tree" ? "secondary" : "ghost"} onClick={() => setMode("tree")}>
                <TableProperties /> Tree
              </Button>
              <Button size="sm" className="h-7" variant={mode === "raw" ? "secondary" : "ghost"} onClick={() => setMode("raw")}>
                <Braces /> JSON
              </Button>
            </div>
          </>
        }
      >
        {mode === "raw" ? (
          <CodeBlock className="max-h-[32rem]">{JSON.stringify(schema, null, 2)}</CodeBlock>
        ) : rows.length === 0 ? (
          <div className="px-4 pb-4">
            <EmptyState icon={ListTree} title="No fields">The schema has no columns.</EmptyState>
          </div>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead className="border-y bg-muted/40 text-left text-xs font-medium tracking-wide text-muted-foreground uppercase">
                <tr>
                  <th className="h-9 min-w-72 px-4">Field</th>
                  <th className="h-9 min-w-40 px-3">Type</th>
                  <th className="h-9 px-3">Nullable</th>
                  <th className="h-9 px-3">Metadata</th>
                </tr>
              </thead>
              <tbody>
                {rows.map((r) => (
                  <tr key={r.path} className="border-b align-top last:border-0 hover:bg-muted/30">
                    <td className="py-2 pr-3 pl-4">
                      <div className="flex items-center gap-1" style={{ paddingLeft: r.depth * 18 }}>
                        {r.hasChildren ? (
                          <button
                            type="button"
                            onClick={() => toggle(r.path)}
                            aria-label={expanded.has(r.path) ? `Collapse ${r.name}` : `Expand ${r.name}`}
                            aria-expanded={expanded.has(r.path)}
                            className="flex h-5 w-5 shrink-0 items-center justify-center rounded-sm text-muted-foreground hover:bg-accent hover:text-foreground"
                          >
                            <ChevronRight className={cn("h-3.5 w-3.5 transition-transform", expanded.has(r.path) && "rotate-90")} />
                          </button>
                        ) : (
                          <span className="w-5 shrink-0" />
                        )}
                        <Identifier name={r.name} className="font-mono text-[13px]" />
                      </div>
                    </td>
                    <td className="px-3 py-2 font-mono text-xs break-words text-muted-foreground" title={r.fullType}>
                      {r.type}
                    </td>
                    <td className="px-3 py-2">
                      <Badge variant="outline">{r.nullable ? "nullable" : "required"}</Badge>
                    </td>
                    <td className="px-3 py-2">
                      {r.metadata.length === 0 ? (
                        <span className="text-muted-foreground/60">—</span>
                      ) : (
                        <div className="flex flex-wrap gap-1">
                          {r.metadata.map(([k, v]) => (
                            <Badge key={k} variant="secondary" className="max-w-64 font-mono font-normal">
                              <span className="truncate" title={`${k}=${v}`}>
                                {k}=<span className="text-muted-foreground">{v}</span>
                              </span>
                            </Badge>
                          ))}
                        </div>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Section>

      <Section title="Schema metadata">
        <PropertiesList entries={schemaMeta} empty="No schema-level metadata." />
      </Section>
    </div>
  );
}
