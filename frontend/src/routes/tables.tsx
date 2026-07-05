import { useState } from "react";
import { createFileRoute, Link } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import {
  flexRender,
  getCoreRowModel,
  getFilteredRowModel,
  getPaginationRowModel,
  getSortedRowModel,
  useReactTable,
  type ColumnDef,
  type SortingState,
} from "@tanstack/react-table";
import { Plus, ArrowUpDown, ChevronLeft, ChevronRight } from "lucide-react";
import { listTables } from "@/lib/api";
import type { TableEntry } from "@/lib/schemas";
import { formatBytes, formatRelative, nsToString, ttlPolicyString } from "@/lib/format";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { EmptyState } from "@/components/empty-state";
import { ErrorState } from "@/components/error-state";
import { RegisterDialog } from "@/components/register-dialog";

export const Route = createFileRoute("/tables")({
  component: TablesPage,
});

const columns: ColumnDef<TableEntry>[] = [
  {
    accessorKey: "id",
    header: "Table",
    cell: ({ row }) => (
      <Link
        to="/tables/$tableId"
        params={{ tableId: row.original.id }}
        className="font-medium text-primary hover:underline"
      >
        {row.original.id}
      </Link>
    ),
  },
  {
    accessorFn: (r) => nsToString(r.namespace),
    id: "namespace",
    header: "Namespace",
    cell: ({ row }) => {
      const ns = nsToString(row.original.namespace);
      return ns ? (
        <Link to="/namespaces/$ns" params={{ ns }} className="text-muted-foreground hover:underline">
          {ns}
        </Link>
      ) : <span className="text-muted-foreground">—</span>;
    },
  },
  {
    accessorFn: (r) => r.versions.length,
    id: "versions",
    header: "Versions",
    cell: ({ row }) => <span className="tabular-nums">{row.original.versions.length}</span>,
  },
  {
    accessorFn: (r) => r.versions.reduce((s, v) => s + v.storage_bytes_total, 0),
    id: "bytes",
    header: "Total size",
    cell: ({ row }) => {
      const total = row.original.versions.reduce((s, v) => s + v.storage_bytes_total, 0);
      return <span className="tabular-nums text-muted-foreground">{formatBytes(total)}</span>;
    },
  },
  {
    accessorKey: "owner",
    header: "Owner",
    cell: ({ row }) => <span className="text-muted-foreground">{row.original.owner ?? "—"}</span>,
  },
  {
    accessorFn: (r) => (r.ttl_policy ? ttlPolicyString(r.ttl_policy) : ""),
    id: "ttl",
    header: "TTL",
    cell: ({ row }) =>
      row.original.ttl_policy ? (
        <Badge variant="secondary">{ttlPolicyString(row.original.ttl_policy)}</Badge>
      ) : <span className="text-muted-foreground">none</span>,
  },
  {
    accessorKey: "last_swept",
    header: "Swept",
    cell: ({ row }) => (
      <span className="text-muted-foreground">{formatRelative(row.original.last_swept)}</span>
    ),
  },
];

function TablesPage() {
  const { data, isLoading, error } = useQuery({
    queryKey: ["tables"],
    queryFn: listTables,
  });
  const [globalFilter, setGlobalFilter] = useState("");
  const [sorting, setSorting] = useState<SortingState>([]);
  const [registerOpen, setRegisterOpen] = useState(false);

  const rows = data ?? [];

  const table = useReactTable({
    data: rows,
    columns,
    state: { globalFilter, sorting },
    onGlobalFilterChange: setGlobalFilter,
    onSortingChange: setSorting,
    getCoreRowModel: getCoreRowModel(),
    getFilteredRowModel: getFilteredRowModel(),
    getSortedRowModel: getSortedRowModel(),
    getPaginationRowModel: getPaginationRowModel(),
    initialState: { pagination: { pageSize: 50 } },
  });

  const { rows: modelRows } = table.getRowModel();

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between gap-4">
        <h1 className="text-2xl font-semibold">Tables</h1>
        <Button size="sm" onClick={() => setRegisterOpen(true)}>
          <Plus className="h-4 w-4" /> Register table
        </Button>
      </div>

      <div className="flex items-center gap-2">
        <Input
          placeholder="Search tables, namespaces, owners…"
          value={globalFilter}
          onChange={(e) => setGlobalFilter(e.target.value)}
          className="max-w-sm"
        />
        <span className="text-sm text-muted-foreground">{rows.length} tables</span>
      </div>

      {error ? (
        <ErrorState message={(error as Error).message} />
      ) : isLoading ? (
        <div className="py-8 text-center text-sm text-muted-foreground">Loading…</div>
      ) : rows.length === 0 ? (
        <EmptyState>No tables registered.</EmptyState>
      ) : (
        <div className="rounded-lg border">
          <table className="w-full text-sm">
            <thead className="border-b bg-muted/40">
              <tr>
                {table.getFlatHeaders().map((header) => (
                  <th
                    key={header.id}
                    className="px-3 py-2 text-left text-xs font-medium uppercase tracking-wide text-muted-foreground"
                  >
                    <button
                      type="button"
                      className="inline-flex items-center gap-1 hover:text-foreground"
                      onClick={header.column.getToggleSortingHandler()}
                    >
                      {flexRender(header.column.columnDef.header, header.getContext())}
                      <ArrowUpDown className="h-3 w-3 opacity-50" />
                    </button>
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {modelRows.map((row) => (
                <tr key={row.id} className="border-b last:border-0 hover:bg-muted/40">
                  {row.getVisibleCells().map((cell) => (
                    <td key={cell.id} className="px-3 py-2 align-middle">
                      {flexRender(cell.column.columnDef.cell, cell.getContext())}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {rows.length > 0 && (
        <div className="flex items-center justify-between text-sm">
          <span className="text-muted-foreground">
            Page {table.getState().pagination.pageIndex + 1} of {table.getPageCount()}
          </span>
          <div className="flex gap-2">
            <Button
              size="sm"
              variant="outline"
              onClick={() => table.previousPage()}
              disabled={!table.getCanPreviousPage()}
            >
              <ChevronLeft className="h-4 w-4" /> Prev
            </Button>
            <Button
              size="sm"
              variant="outline"
              onClick={() => table.nextPage()}
              disabled={!table.getCanNextPage()}
            >
              Next <ChevronRight className="h-4 w-4" />
            </Button>
          </div>
        </div>
      )}

      <RegisterDialog open={registerOpen} onOpenChange={setRegisterOpen} />
    </div>
  );
}
