import { useMemo, useState } from "react";
import type { UseQueryResult } from "@tanstack/react-query";
import { Braces, ChevronRight, ListTree, TableProperties } from "lucide-react";
import { useColumnStorage } from "@/lib/catalog-queries";
import { formatBytes, formatCountOf, formatPercent } from "@/lib/format";
import { expandablePaths, flattenSchema, schemaMetadataByColumn, type SchemaRow } from "@/lib/schema-tree";
import { sortedEntries } from "@/lib/table-metadata";
import type { ColumnStorageResponse, DescribeTableResponse, JsonArrowSchema } from "@/lib/schemas";
import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { CodeBlock } from "@/components/code-block";
import { Dash, EmptyState } from "@/components/empty-state";
import { ErrorState } from "@/components/error-state";
import { HelpTooltip } from "@/components/help-tooltip";
import { Identifier } from "@/components/identifier";
import { JsonTree } from "@/components/json-tree";
import { PropertiesList } from "@/components/properties-list";
import { Section } from "@/components/section";
import { Fact, Mono, PaneSection, SidePane, SidePaneLayout, usePaneSelection } from "@/components/side-pane";
import { STICKY_HEAD } from "./sticky-head";

/** Schema-level metadata, then the field table; selecting a field opens its details beside the
 *  table. Per-column size on disk loads once the schema is known. */
export function SchemaTab({
  segments,
  detail,
}: {
  segments: readonly string[];
  detail: UseQueryResult<DescribeTableResponse>;
}) {
  const columns = useColumnStorage(segments, !detail.isPending && !!detail.data?.schema);
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

const rowPath = (r: SchemaRow) => r.path;

/** Bytes per top-level column, plus the total and largest for shares and bar scaling. */
interface ColumnSizes {
  bytes: Map<string, number>;
  total: number;
  max: number;
}

function columnSizes(columns: ColumnStorageResponse | undefined): ColumnSizes {
  const bytes = new Map(columns?.columns.map((c) => [c.name, c.bytes_on_disk]));
  const sizes = [...bytes.values()];
  return { bytes, total: sizes.reduce((s, b) => s + b, 0), max: Math.max(0, ...sizes) };
}

function SizeBar({ bytes, max }: { bytes: number; max: number }) {
  return (
    <div className="relative h-5 w-40 overflow-hidden rounded bg-muted">
      <div
        className="absolute inset-y-0 left-0 bg-[hsl(var(--chart-1)/0.35)]"
        style={{ width: `${max ? (bytes / max) * 100 : 0}%` }}
      />
      <span className="relative px-1.5 text-xs leading-5 tabular-nums">{formatBytes(bytes)}</span>
    </div>
  );
}

function SizeCell({ row, columns, sizes }: { row: SchemaRow; columns: UseQueryResult<ColumnStorageResponse>; sizes: ColumnSizes }) {
  if (row.depth > 0) return <Dash />;
  if (columns.error) {
    return (
      <span className="text-xs text-muted-foreground" title={columns.error.message}>
        Unavailable
      </span>
    );
  }
  if (!columns.data) return <Skeleton className="h-5 w-40" />;
  const bytes = sizes.bytes.get(row.name);
  return bytes == null ? <Dash /> : <SizeBar bytes={bytes} max={sizes.max} />;
}

function SizeFacts({ row, columns, sizes }: { row: SchemaRow; columns: UseQueryResult<ColumnStorageResponse>; sizes: ColumnSizes }) {
  if (row.depth > 0) {
    return (
      <span className="text-sm text-muted-foreground">
        Counted in its top-level column <Mono>{row.path.split(".")[0]}</Mono>.
      </span>
    );
  }
  if (columns.error) return <ErrorState message={columns.error.message} />;
  if (!columns.data) return <Skeleton className="h-4 w-40" />;
  const bytes = sizes.bytes.get(row.name);
  if (bytes == null) return <span className="text-sm text-muted-foreground">Not measured</span>;
  const rank = [...sizes.bytes.values()].filter((b) => b > bytes).length + 1;
  return (
    <>
      <Fact label="Size">
        <span className="tabular-nums">{formatBytes(bytes)}</span>
      </Fact>
      <Fact label="Share of all columns">
        <span className="tabular-nums">{formatPercent(sizes.total ? bytes / sizes.total : null)}</span>
      </Fact>
      <Fact label="Rank">
        <span className="tabular-nums">
          {rank} of {sizes.bytes.size}
        </span>
      </Fact>
    </>
  );
}

function FieldPane({
  row,
  columns,
  sizes,
  schemaEntries,
  onClose,
}: {
  row: SchemaRow;
  columns: UseQueryResult<ColumnStorageResponse>;
  sizes: ColumnSizes;
  schemaEntries: [string, string][];
  onClose: () => void;
}) {
  return (
    <SidePane
      label={`Field ${row.path} details`}
      onClose={onClose}
      bodyKey={row.path}
      header={
        <>
          <Identifier name={row.name} className="font-mono font-medium" />
          <div className="font-mono text-xs text-muted-foreground">{row.type}</div>
        </>
      }
    >
      <PaneSection title="Field">
        <Fact label="Path">
          <Mono>{row.path}</Mono>
        </Fact>
        <Fact label="Type">
          <Mono>{row.fullType}</Mono>
        </Fact>
        <Fact label="Nullable">{row.nullable ? "Yes" : "No"}</Fact>
      </PaneSection>
      <PaneSection title="Size on disk">
        <SizeFacts row={row} columns={columns} sizes={sizes} />
      </PaneSection>
      <PaneSection title="Field metadata">
        <PropertiesList entries={row.metadata} empty="No field metadata." />
      </PaneSection>
      {schemaEntries.length > 0 && (
        <PaneSection title="Schema metadata for this column">
          <PropertiesList entries={schemaEntries} empty="" />
        </PaneSection>
      )}
      <PaneSection title="Arrow field">
        <div className="overflow-x-auto">
          <JsonTree name={row.name} value={row.field} />
        </div>
      </PaneSection>
    </SidePane>
  );
}

function SchemaView({ schema, columns }: { schema: JsonArrowSchema; columns: UseQueryResult<ColumnStorageResponse> }) {
  const [mode, setMode] = useState<"tree" | "raw">("tree");
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set());
  const allPaths = useMemo(() => expandablePaths(schema.fields), [schema]);
  const rows = useMemo(() => flattenSchema(schema.fields, expanded), [schema, expanded]);
  const schemaEntries = useMemo(() => sortedEntries(schema.metadata), [schema]);
  const byColumn = useMemo(
    () =>
      schemaMetadataByColumn(
        schema.metadata,
        schema.fields.map((f) => f.name),
      ),
    [schema],
  );
  const sizes = useMemo(() => columnSizes(columns.data), [columns.data]);
  const { current, close, rowProps } = usePaneSelection(rows, rowPath);

  function toggle(path: string) {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (!next.delete(path)) next.add(path);
      return next;
    });
  }

  return (
    <div className="space-y-6">
      <Section title="Schema metadata" help="Key/values on the Arrow schema itself.">
        <PropertiesList entries={schemaEntries} empty="No schema-level metadata." />
      </Section>

      <SidePaneLayout
        pane={
          current && (
            <FieldPane
              row={current}
              columns={columns}
              sizes={sizes}
              schemaEntries={current.depth === 0 ? (byColumn.get(current.name) ?? []) : []}
              onClose={close}
            />
          )
        }
      >
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
            <table className="w-full text-sm">
              <thead>
                <tr>
                  <th className={cn(STICKY_HEAD, "min-w-48 pl-4")}>Field</th>
                  <th className={cn(STICKY_HEAD, "min-w-32")}>Type</th>
                  {!current && <th className={STICKY_HEAD}>Nullable</th>}
                  {!current && <th className={STICKY_HEAD}>Field metadata</th>}
                  <th className={cn(STICKY_HEAD, "pr-4")}>
                    <span className="inline-flex items-center gap-1">
                      Size on disk
                      <HelpTooltip label="About size on disk">
                        Current version&apos;s data files, scaled to the largest column. Nested fields count toward
                        their column; blob payloads and index files are excluded.
                      </HelpTooltip>
                    </span>
                  </th>
                </tr>
              </thead>
              <tbody>
                {rows.map((r) => (
                  <tr
                    key={r.path}
                    {...rowProps(r.path, { selfOnly: true })}
                    aria-selected={r.path === current?.path}
                    className={cn(
                      "cursor-pointer border-b align-middle outline-none last:border-0 focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset",
                      r.path === current?.path ? "bg-accent" : "hover:bg-muted/30",
                    )}
                  >
                    <td className="py-2 pr-3 pl-4">
                      <div className="flex items-center gap-1" style={{ paddingLeft: r.depth * 18 }}>
                        {r.hasChildren ? (
                          <button
                            type="button"
                            onClick={(e) => {
                              e.stopPropagation();
                              toggle(r.path);
                            }}
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
                    {!current && (
                      <td className="px-3 py-2">
                        <Badge variant="outline">{r.nullable ? "nullable" : "required"}</Badge>
                      </td>
                    )}
                    {!current && (
                      <td className="px-3 py-2 whitespace-nowrap">
                        {r.metadata.length ? (
                          <Badge variant="secondary" className="font-normal">
                            {formatCountOf(r.metadata.length, "entry", "entries")}
                          </Badge>
                        ) : (
                          <Dash />
                        )}
                      </td>
                    )}
                    <td className="px-3 py-2 pr-4">
                      <SizeCell row={r} columns={columns} sizes={sizes} />
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </Section>
      </SidePaneLayout>
    </div>
  );
}
