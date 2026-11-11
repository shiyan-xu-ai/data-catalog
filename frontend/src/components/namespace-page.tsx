import { Link } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { Folder, Table2 } from "lucide-react";
import { describeNamespace } from "@/lib/api";
import {
  keys,
  useChildNamespaces,
  useNamespaceTables,
  useTableCheap,
  type PagedNames,
} from "@/lib/catalog-queries";
import { joinId } from "@/lib/ids";
import { DESCRIPTION, OWNER, REGISTERED_AT, UPDATED_AT } from "@/lib/table-metadata";
import { useInView } from "@/lib/use-in-view";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { EmptyState } from "@/components/empty-state";
import { ErrorState } from "@/components/error-state";
import { PropertiesList } from "@/components/properties-list";
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

/** Namespace page; the catalog root when `segments` is empty. Read-only: namespaces and tables
 *  are created and registered through the API. Tables live only in nested namespaces. */
export function NamespacePage({ segments }: { segments: readonly string[] }) {
  const isRoot = segments.length === 0;
  const id = joinId(segments);
  const describe = useQuery({
    queryKey: keys.namespace(id),
    queryFn: () => describeNamespace(segments),
    enabled: !isRoot,
  });
  const children = useChildNamespaces(segments);
  const tables = useNamespaceTables(segments);
  const name = isRoot ? "Catalog" : segments[segments.length - 1];
  const nsEntries = Object.entries(describe.data?.properties ?? {}).sort(([a], [b]) => a.localeCompare(b));

  return (
    <div className="space-y-6">
      <div className="flex min-w-0 items-center gap-3">
        <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border bg-muted/50">
          <Folder className="h-5 w-5 text-muted-foreground" />
        </div>
        <h1 className="min-w-0 truncate text-xl font-semibold tracking-tight">{name}</h1>
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
          description="Catalog-recorded fields only"
          help="Open a table to see its Lance details."
          contentClassName="px-0 pb-0"
        >
          <ListStatus query={tables}>
            {tables.names.length === 0 ? (
              <div className="px-4 pb-4">
                <EmptyState icon={Table2} title="No tables">
                  No tables are registered in this namespace.
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
              {isRoot ? "No namespaces exist yet." : "This namespace has no children."}
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
    </div>
  );
}
