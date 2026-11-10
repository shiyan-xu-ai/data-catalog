import { useState } from "react";
import { Link, useNavigate } from "@tanstack/react-router";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Folder, FolderPlus, Table2, Trash2, Upload } from "lucide-react";
import { toast } from "sonner";
import { ApiError, describeNamespace, dropNamespace } from "@/lib/api";
import {
  keys,
  useChildNamespaces,
  useNamespaceTables,
  useTableCheap,
  type PagedNames,
} from "@/lib/catalog-queries";
import { displayName, joinId } from "@/lib/ids";
import { ErrorCode } from "@/lib/schemas";
import { DESCRIPTION, OWNER, REGISTERED_AT, UPDATED_AT } from "@/lib/table-metadata";
import { useInView } from "@/lib/use-in-view";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { ConfirmDialog } from "@/components/confirm-dialog";
import { CreateNamespaceDialog } from "@/components/create-namespace-dialog";
import { EmptyState } from "@/components/empty-state";
import { ErrorState } from "@/components/error-state";
import { PropertiesList } from "@/components/properties-list";
import { RegisterDialog } from "@/components/register-dialog";
import { Section } from "@/components/section";
import { Timestamp } from "@/components/timestamp";

function ListStatus({ query, children }: { query: PagedNames; children: React.ReactNode }) {
  if (query.error) return <ErrorState message={query.error.message} />;
  if (query.isLoading) {
    return (
      <div className="space-y-2">
        <Skeleton className="h-8 w-full" />
        <Skeleton className="h-8 w-full" />
        <Skeleton className="h-8 w-2/3" />
      </div>
    );
  }
  return (
    <>
      {children}
      {query.hasNextPage && (
        <div className="pt-3 text-center">
          <Button size="sm" variant="outline" disabled={query.isFetchingNextPage} onClick={() => query.fetchNextPage()}>
            {query.isFetchingNextPage ? "Loading…" : "Load more"}
          </Button>
        </div>
      )}
    </>
  );
}

/** One table row. Catalog fields come from a cheap describe (Postgres only), fetched once the
 *  row scrolls into view. */
function TableListRow({ segments }: { segments: string[] }) {
  const [ref, inView] = useInView<HTMLTableRowElement>();
  const { data, error, isLoading } = useTableCheap(segments, inView);
  const props = data?.properties ?? {};
  const pending = !inView || isLoading;
  const cell = (content: React.ReactNode, width = "w-24") =>
    pending ? <Skeleton className={`h-4 ${width}`} /> : error ? <span className="text-muted-foreground">—</span> : content;
  const location = data?.location ?? data?.table_uri;
  return (
    <TableRow ref={ref}>
      <TableCell className="max-w-64">
        <Link
          to="/tables/$tableId"
          params={{ tableId: joinId(segments) }}
          className="flex items-center gap-2 font-mono text-[13px] font-medium hover:underline"
        >
          <Table2 className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
          <span className="truncate">{segments[segments.length - 1]}</span>
        </Link>
      </TableCell>
      <TableCell className="max-w-80">
        {error ? (
          <span className="text-xs text-destructive" title={error.message}>Failed to load details</span>
        ) : (
          cell(
            props[DESCRIPTION] ? (
              <span className="line-clamp-2 text-muted-foreground" title={props[DESCRIPTION]}>{props[DESCRIPTION]}</span>
            ) : (
              <span className="text-muted-foreground/60">—</span>
            ),
            "w-40",
          )
        )}
      </TableCell>
      <TableCell className="whitespace-nowrap">{cell(props[OWNER] ?? <span className="text-muted-foreground/60">—</span>)}</TableCell>
      <TableCell className="max-w-72">
        {cell(
          <span className="block truncate font-mono text-xs text-muted-foreground" title={location ?? undefined}>
            {location ?? "—"}
          </span>,
          "w-48",
        )}
      </TableCell>
      <TableCell className="whitespace-nowrap text-muted-foreground">{cell(<Timestamp value={props[REGISTERED_AT]} />, "w-20")}</TableCell>
      <TableCell className="whitespace-nowrap text-muted-foreground">{cell(<Timestamp value={props[UPDATED_AT]} />, "w-20")}</TableCell>
    </TableRow>
  );
}

/** Namespace page; the catalog root when `segments` is empty. Tables live only in nested
 *  namespaces (the root has no catalog row), so table actions are hidden at the root. */
export function NamespacePage({ segments }: { segments: readonly string[] }) {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const [createOpen, setCreateOpen] = useState(false);
  const [registerOpen, setRegisterOpen] = useState(false);
  const isRoot = segments.length === 0;
  const id = joinId(segments);
  const parent = segments.slice(0, -1);

  const describe = useQuery({
    queryKey: keys.namespace(id),
    queryFn: () => describeNamespace(segments),
    enabled: !isRoot,
  });
  const children = useChildNamespaces(segments);
  const tables = useNamespaceTables(segments);

  async function drop() {
    try {
      await dropNamespace(segments);
    } catch (e) {
      if (e instanceof ApiError && e.code === ErrorCode.NamespaceNotEmpty) {
        toast.error(
          `"${displayName(segments)}" is not empty. Drop its child namespaces and deregister its tables (through the API) first.`,
        );
      } else {
        toast.error((e as Error).message);
      }
      throw e;
    }
    toast.success(`Dropped namespace "${displayName(segments)}"`);
    qc.removeQueries({ queryKey: keys.namespace(id) });
    qc.invalidateQueries({ queryKey: keys.namespaces(joinId(parent)) });
    qc.invalidateQueries({ queryKey: keys.allNamespaces });
    navigate(parent.length ? { to: "/namespaces/$nsId", params: { nsId: joinId(parent) } } : { to: "/namespaces" });
  }

  const name = isRoot ? "Catalog" : segments[segments.length - 1];
  const nsEntries = Object.entries(describe.data?.properties ?? {}).sort(([a], [b]) => a.localeCompare(b));

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div className="flex min-w-0 items-start gap-3">
          <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg border bg-muted/50">
            <Folder className="h-5 w-5 text-muted-foreground" />
          </div>
          <div className="min-w-0">
            <h1 className="truncate text-xl font-semibold tracking-tight">{name}</h1>
            <p className="truncate text-sm text-muted-foreground">
              {isRoot ? "Top-level namespaces" : <span className="font-mono">{displayName(segments)}</span>}
            </p>
          </div>
        </div>
        <div className="flex flex-wrap gap-2">
          <Button variant="outline" onClick={() => setCreateOpen(true)}>
            <FolderPlus /> New namespace
          </Button>
          {!isRoot && (
            <>
              <Button onClick={() => setRegisterOpen(true)}>
                <Upload /> Register table
              </Button>
              <ConfirmDialog
                trigger={
                  <Button variant="outline" className="text-destructive hover:text-destructive">
                    <Trash2 /> Drop
                  </Button>
                }
                title="Drop namespace"
                description={
                  <>
                    Drop <span className="font-mono text-foreground">{displayName(segments)}</span>? Only
                    empty namespaces can be dropped.
                  </>
                }
                confirmLabel="Drop namespace"
                destructive
                onConfirm={drop}
              />
            </>
          )}
        </div>
      </div>

      {describe.error && <ErrorState message={describe.error.message} />}

      {!isRoot && (
        <Section title="Properties">
          {describe.isLoading ? (
            <Skeleton className="h-4 w-48" />
          ) : (
            <PropertiesList entries={nsEntries} empty="No properties." />
          )}
        </Section>
      )}

      {!isRoot && (
        <Section
          title="Tables"
          description="Catalog-recorded fields only; open a table for its Lance details."
          contentClassName="px-0 pb-0"
        >
          <ListStatus query={tables}>
            {tables.names.length === 0 ? (
              <div className="px-4 pb-4">
                <EmptyState icon={Table2} title="No tables">
                  Register an existing Lance dataset to add one.
                </EmptyState>
              </div>
            ) : (
              <Table>
                <TableHeader className="bg-muted/40">
                  <TableRow className="hover:bg-transparent">
                    <TableHead className="pl-4">Name</TableHead>
                    <TableHead>Description</TableHead>
                    <TableHead>Owner</TableHead>
                    <TableHead>Location</TableHead>
                    <TableHead>Registered</TableHead>
                    <TableHead>Updated</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody className="[&_td:first-child]:pl-4">
                  {tables.names.map((t) => (
                    <TableListRow key={t} segments={[...segments, t]} />
                  ))}
                </TableBody>
              </Table>
            )}
          </ListStatus>
        </Section>
      )}

      <Section title={isRoot ? "Namespaces" : "Child namespaces"}>
        <ListStatus query={children}>
          {children.names.length === 0 ? (
            <EmptyState icon={Folder} title="No namespaces">
              {isRoot ? "Create a namespace to start registering tables." : "This namespace has no children."}
            </EmptyState>
          ) : (
            <ul className="grid grid-cols-[repeat(auto-fill,minmax(14rem,1fr))] gap-2">
              {children.names.map((child) => (
                <li key={child}>
                  <Link
                    to="/namespaces/$nsId"
                    params={{ nsId: joinId([...segments, child]) }}
                    className="flex items-center gap-2 rounded-md border bg-background px-3 py-2 text-sm transition-colors hover:border-foreground/20 hover:bg-accent"
                  >
                    <Folder className="h-4 w-4 shrink-0 text-muted-foreground" />
                    <span className="truncate">{child}</span>
                  </Link>
                </li>
              ))}
            </ul>
          )}
        </ListStatus>
      </Section>

      <CreateNamespaceDialog parent={segments} open={createOpen} onOpenChange={setCreateOpen} />
      {!isRoot && <RegisterDialog namespace={segments} open={registerOpen} onOpenChange={setRegisterOpen} />}
    </div>
  );
}
