import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Braces, TableProperties } from "lucide-react";
import type { AuxEntry } from "@/lib/schemas";
import { sampleAux } from "@/lib/api";
import { formatBytes } from "@/lib/format";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { CodeBlock, prettyJson } from "@/components/code-block";
import { ErrorState } from "@/components/error-state";
import { EmptyState } from "@/components/empty-state";

/** A table can only be sampled when it is a lance dataset or a parquet directory. */
function isSampleable(a: AuxEntry): boolean {
  return a.format === "lance" || a.format === "parquet";
}

function categoryBadge(category: string | null) {
  if (category === "nested_sidecar") return <Badge variant="secondary">nested sidecar</Badge>;
  if (category === "sidecar") return <Badge variant="outline">sidecar</Badge>;
  return <span className="text-muted-foreground">—</span>;
}

/** Enriched auxiliary-table listing with per-entry schema + sample viewers. */
export function AuxTable({
  tableId,
  versionId,
  aux,
}: {
  tableId: string;
  versionId: string;
  aux: AuxEntry[];
}) {
  return (
    <div className="overflow-x-auto">
      <table className="w-full text-sm">
        <thead className="border-b bg-muted/40 text-left text-xs uppercase tracking-wide text-muted-foreground">
          <tr>
            <th className="px-3 py-2">Name</th>
            <th className="px-3 py-2">Category</th>
            <th className="px-3 py-2">Format</th>
            <th className="px-3 py-2 text-right">Rows</th>
            <th className="px-3 py-2 text-right">Size</th>
            <th className="px-3 py-2">Writer</th>
            <th className="px-3 py-2 text-right">Actions</th>
          </tr>
        </thead>
        <tbody>
          {aux.map((a) => (
            <tr key={a.name} className="border-b align-top last:border-0">
              <td className="max-w-md px-3 py-2 font-mono text-xs break-all">{a.name}</td>
              <td className="px-3 py-2">{categoryBadge(a.category)}</td>
              <td className="px-3 py-2">
                <Badge variant="outline">{a.format}</Badge>
              </td>
              <td className="px-3 py-2 text-right tabular-nums">
                {a.row_count != null ? a.row_count.toLocaleString() : "—"}
              </td>
              <td className="px-3 py-2 text-right tabular-nums">{formatBytes(a.storage_bytes)}</td>
              <td className="px-3 py-2 text-xs text-muted-foreground">{a.writer_version ?? "—"}</td>
              <td className="px-3 py-2">
                <div className="flex justify-end gap-1">
                  {a.schema_json && <SchemaDialog name={a.name} schemaJson={a.schema_json} />}
                  {isSampleable(a) && (
                    <SampleDialog tableId={tableId} versionId={versionId} aux={a} />
                  )}
                </div>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function SchemaDialog({ name, schemaJson }: { name: string; schemaJson: string }) {
  return (
    <Dialog>
      <DialogTrigger asChild>
        <Button size="sm" variant="ghost">
          <Braces className="h-4 w-4" /> Schema
        </Button>
      </DialogTrigger>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>Schema</DialogTitle>
          <DialogDescription className="font-mono text-xs break-all">{name}</DialogDescription>
        </DialogHeader>
        <CodeBlock>{prettyJson(schemaJson)}</CodeBlock>
      </DialogContent>
    </Dialog>
  );
}

function SampleDialog({
  tableId,
  versionId,
  aux,
}: {
  tableId: string;
  versionId: string;
  aux: AuxEntry;
}) {
  const [open, setOpen] = useState(false);
  const [limit, setLimit] = useState(20);
  const query = useQuery({
    queryKey: ["aux-sample", tableId, versionId, aux.name, limit],
    queryFn: () => sampleAux(tableId, versionId, aux.name, limit),
    enabled: open,
  });

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>
        <Button size="sm" variant="outline">
          <TableProperties className="h-4 w-4" /> Sample
        </Button>
      </DialogTrigger>
      <DialogContent className="max-w-4xl">
        <DialogHeader>
          <DialogTitle>Sample rows</DialogTitle>
          <DialogDescription className="font-mono text-xs break-all">{aux.name}</DialogDescription>
        </DialogHeader>

        <div className="flex items-center gap-2 text-sm">
          <span className="text-muted-foreground">Rows</span>
          <Select value={String(limit)} onValueChange={(v) => setLimit(Number(v))}>
            <SelectTrigger className="h-8 w-24">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {[20, 50, 100].map((n) => (
                <SelectItem key={n} value={String(n)}>
                  {n}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>

        {query.isLoading && (
          <div className="py-8 text-center text-sm text-muted-foreground">Loading sample…</div>
        )}
        {query.error && <ErrorState message={(query.error as Error).message} />}
        {query.data && <SampleGrid rows={query.data.rows} schema={query.data.schema} />}
      </DialogContent>
    </Dialog>
  );
}

/** Render sampled rows as a scrollable grid; columns follow the sample schema order. */
function SampleGrid({
  rows,
  schema,
}: {
  rows: Record<string, unknown>[];
  schema: Record<string, unknown>[];
}) {
  if (rows.length === 0) return <EmptyState>No rows.</EmptyState>;
  const columns =
    schema.length > 0
      ? schema.map((f) => String(f.name))
      : Object.keys(rows[0] ?? {});

  return (
    <div className="max-h-112 overflow-auto rounded-md border">
      <table className="w-full text-xs">
        <thead className="sticky top-0 border-b bg-muted text-left uppercase tracking-wide text-muted-foreground">
          <tr>
            {columns.map((c) => (
              <th key={c} className="px-2 py-1.5 font-medium">
                {c}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row, i) => (
            <tr key={i} className="border-b last:border-0">
              {columns.map((c) => (
                <td key={c} className="max-w-xs truncate px-2 py-1 font-mono">
                  {renderCell(row[c])}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function renderCell(v: unknown): string {
  if (v == null) return "";
  if (typeof v === "object") return JSON.stringify(v);
  return String(v);
}
