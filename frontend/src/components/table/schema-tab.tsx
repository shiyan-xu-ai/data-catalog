import { useMemo, useState } from "react";
import type { UseQueryResult } from "@tanstack/react-query";
import { Braces, ChevronRight, ListTree, TableProperties } from "lucide-react";
import { useColumnStorage } from "@/lib/catalog-queries";
import { formatBytes, formatPercent } from "@/lib/format";
import { expandablePaths, flattenSchema, schemaMetadataByColumn } from "@/lib/schema-tree";
import type { ColumnStorageResponse, DescribeTableResponse, JsonArrowSchema } from "@/lib/schemas";
import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { CodeBlock } from "@/components/code-block";
import { EmptyState } from "@/components/empty-state";
import { ErrorState } from "@/components/error-state";
import { HelpTooltip } from "@/components/help-tooltip";
import { Identifier } from "@/components/identifier";
import { Section } from "@/components/section";

/** The schema as one table; per-column size on disk loads on its own once the tab opens. */
export function SchemaTab({
  segments,
  detail,
}: {
  segments: readonly string[];
  detail: UseQueryResult<DescribeTableResponse>;
}) {
  const columns = useColumnStorage(segments, true);
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
  return <SchemaView schema={schema} columns={columns} />;
}

const Dash = () => <span className="text-muted-foreground/60">—</span>;

function MetadataBadges({ entries }: { entries: [string, string][] | undefined }) {
  if (!entries?.length) return <Dash />;
  return (
    <div className="flex flex-wrap gap-1">
      {entries.map(([k, v]) => (
        <Badge key={k} variant="secondary" className="max-w-64 font-mono font-normal">
          <span className="truncate" title={`${k}=${v}`}>
            {k}=<span className="text-muted-foreground">{v}</span>
          </span>
        </Badge>
      ))}
    </div>
  );
}

/** Bytes on disk of one top-level column, with its share of all columns. */
function ColumnSize({ name, columns }: { name: string; columns: UseQueryResult<ColumnStorageResponse> }) {
  if (columns.error) {
    return (
      <span className="text-xs text-muted-foreground" title={columns.error.message}>
        Unavailable
      </span>
    );
  }
  if (!columns.data) return <Skeleton className="ml-auto h-4 w-16" />;
  const total = columns.data.columns.reduce((s, c) => s + c.bytes_on_disk, 0);
  const bytes = columns.data.columns.find((c) => c.name === name)?.bytes_on_disk;
  if (bytes == null) return <Dash />;
  return (
    <>
      <div>{formatBytes(bytes)}</div>
      <div className="text-xs text-muted-foreground">{formatPercent(total ? bytes / total : null)}</div>
    </>
  );
}

function SchemaView({ schema, columns }: { schema: JsonArrowSchema; columns: UseQueryResult<ColumnStorageResponse> }) {
  const [mode, setMode] = useState<"tree" | "raw">("tree");
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set());
  const allPaths = useMemo(() => expandablePaths(schema.fields), [schema]);
  const rows = useMemo(() => flattenSchema(schema.fields, expanded), [schema, expanded]);
  const schemaMeta = useMemo(
    () =>
      schemaMetadataByColumn(
        schema.metadata,
        schema.fields.map((f) => f.name),
      ),
    [schema],
  );

  function toggle(path: string) {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (!next.delete(path)) next.add(path);
      return next;
    });
  }

  return (
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
                <th className="h-9 px-3 text-right whitespace-nowrap">
                  <span className="inline-flex items-center gap-1">
                    Size on disk
                    <HelpTooltip label="About size on disk">
                      Served version&apos;s data files; nested fields count toward their column. Excludes blob payloads
                      and index files.
                    </HelpTooltip>
                  </span>
                </th>
                <th className="h-9 px-3 whitespace-nowrap">Field metadata</th>
                <th className="h-9 px-3 whitespace-nowrap">
                  <span className="inline-flex items-center gap-1">
                    Schema metadata
                    <HelpTooltip label="About schema metadata">
                      Schema-level entries keyed &lt;key&gt;:&lt;column&gt;, shown on that column.
                    </HelpTooltip>
                  </span>
                </th>
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
                  <td className="px-3 py-2 text-right whitespace-nowrap tabular-nums">
                    {r.depth === 0 ? <ColumnSize name={r.name} columns={columns} /> : <Dash />}
                  </td>
                  <td className="px-3 py-2">
                    <MetadataBadges entries={r.metadata} />
                  </td>
                  <td className="px-3 py-2">
                    <MetadataBadges entries={r.depth === 0 ? schemaMeta.byColumn.get(r.name) : undefined} />
                  </td>
                </tr>
              ))}
              {schemaMeta.rest.length > 0 && (
                <tr className="align-top">
                  <td colSpan={5} className="py-2 pr-3 pl-4 text-xs text-muted-foreground italic">
                    Not tied to a column
                  </td>
                  <td className="px-3 py-2">
                    <MetadataBadges entries={schemaMeta.rest} />
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      )}
    </Section>
  );
}
