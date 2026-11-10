import { useState } from "react";
import { Link, useNavigate } from "@tanstack/react-router";
import { useInfiniteQuery, useQuery, useQueryClient } from "@tanstack/react-query";
import { Folder, FolderPlus, Table2, Trash2, Upload } from "lucide-react";
import { toast } from "sonner";
import { ApiError, describeNamespace, dropNamespace, listNamespaces, listTables } from "@/lib/api";
import { displayName, joinId } from "@/lib/ids";
import { ErrorCode } from "@/lib/schemas";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { ConfirmDialog } from "@/components/confirm-dialog";
import { CreateNamespaceDialog } from "@/components/create-namespace-dialog";
import { EmptyState } from "@/components/empty-state";
import { ErrorState } from "@/components/error-state";
import { NamespaceBreadcrumb } from "@/components/namespace-breadcrumb";
import { PropertiesList } from "@/components/properties-list";
import { RegisterDialog } from "@/components/register-dialog";

const PAGE_SIZE = 100;

interface Page {
  names: string[];
  page_token?: string | null;
}

interface PagedNames {
  names: string[];
  error: Error | null;
  isLoading: boolean;
  hasNextPage: boolean;
  isFetchingNextPage: boolean;
  fetchNextPage: () => void;
}

/** Spec list endpoint as an infinite query: pages follow `page_token` until it is absent. */
function usePagedNames(
  queryKey: readonly unknown[],
  fetchPage: (token: string | null) => Promise<Page>,
  enabled = true,
): PagedNames {
  const q = useInfiniteQuery({
    queryKey,
    queryFn: ({ pageParam }) => fetchPage(pageParam),
    initialPageParam: null as string | null,
    getNextPageParam: (last) => last.page_token || undefined,
    enabled,
  });
  return {
    names: q.data?.pages.flatMap((p) => p.names) ?? [],
    error: q.error,
    isLoading: q.isLoading,
    hasNextPage: q.hasNextPage,
    isFetchingNextPage: q.isFetchingNextPage,
    fetchNextPage: () => void q.fetchNextPage(),
  };
}

function PagedList({
  title,
  icon: Icon,
  query,
  empty,
  renderLink,
}: {
  title: string;
  icon: typeof Folder;
  query: PagedNames;
  empty: string;
  renderLink: (name: string) => React.ReactNode;
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Icon className="h-4 w-4" /> {title}
        </CardTitle>
      </CardHeader>
      <CardContent>
        {query.error ? (
          <ErrorState message={(query.error as Error).message} />
        ) : query.isLoading ? (
          <div className="py-4 text-center text-sm text-muted-foreground">Loading…</div>
        ) : query.names.length === 0 ? (
          <EmptyState className="py-4">{empty}</EmptyState>
        ) : (
          <ul className="divide-y rounded-md border">
            {query.names.map((name) => (
              <li key={name} className="px-3 py-2 text-sm hover:bg-muted/40">
                {renderLink(name)}
              </li>
            ))}
          </ul>
        )}
        {query.hasNextPage && (
          <div className="pt-3 text-center">
            <Button
              size="sm"
              variant="outline"
              disabled={query.isFetchingNextPage}
              onClick={() => query.fetchNextPage()}
            >
              {query.isFetchingNextPage ? "Loading…" : "Load more"}
            </Button>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

/** Browser for one namespace: the root when `segments` is empty. Tables live only in nested
 *  namespaces (the root has no catalog row), so table actions are hidden at the root. */
export function NamespaceBrowser({ segments }: { segments: readonly string[] }) {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const [createOpen, setCreateOpen] = useState(false);
  const [registerOpen, setRegisterOpen] = useState(false);
  const isRoot = segments.length === 0;
  const id = joinId(segments);
  const parent = segments.slice(0, -1);

  const describe = useQuery({
    queryKey: ["namespace", id],
    queryFn: () => describeNamespace(segments),
    enabled: !isRoot,
  });

  const children = usePagedNames(["namespaces", id], async (token) => {
    const r = await listNamespaces(segments, token, PAGE_SIZE);
    return { names: r.namespaces, page_token: r.page_token };
  });

  const tables = usePagedNames(
    ["tables", id],
    async (token) => {
      const r = await listTables(segments, token, PAGE_SIZE);
      return { names: r.tables, page_token: r.page_token };
    },
    !isRoot,
  );

  async function drop() {
    try {
      await dropNamespace(segments);
    } catch (e) {
      if (e instanceof ApiError && e.code === ErrorCode.NamespaceNotEmpty) {
        toast.error(`"${displayName(segments)}" is not empty. Drop its child namespaces and deregister its tables first.`);
      } else {
        toast.error((e as Error).message);
      }
      throw e;
    }
    toast.success(`Dropped namespace "${displayName(segments)}"`);
    qc.removeQueries({ queryKey: ["namespace", id] });
    qc.invalidateQueries({ queryKey: ["namespaces", joinId(parent)] });
    navigate(parent.length ? { to: "/namespaces/$nsId", params: { nsId: joinId(parent) } } : { to: "/namespaces" });
  }

  if (describe.error) {
    return (
      <div className="space-y-4">
        <NamespaceBreadcrumb segments={parent} />
        <ErrorState message={(describe.error as Error).message} />
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-end justify-between gap-4">
        <div className="space-y-1">
          {!isRoot && <NamespaceBreadcrumb segments={parent} />}
          <h1 className={isRoot ? "text-2xl font-semibold" : "font-mono text-2xl"}>
            {isRoot ? "Namespaces" : displayName(segments)}
          </h1>
        </div>
        <div className="flex flex-wrap gap-2">
          <Button variant="outline" onClick={() => setCreateOpen(true)}>
            <FolderPlus /> Create namespace
          </Button>
          {!isRoot && (
            <>
              <Button onClick={() => setRegisterOpen(true)}>
                <Upload /> Register table
              </Button>
              <ConfirmDialog
                trigger={
                  <Button variant="destructive">
                    <Trash2 /> Drop
                  </Button>
                }
                title="Drop namespace"
                description={
                  <>
                    Drop <span className="font-mono">{displayName(segments)}</span>? Only empty
                    namespaces can be dropped.
                  </>
                }
                confirmLabel="Drop"
                destructive
                onConfirm={drop}
              />
            </>
          )}
        </div>
      </div>

      {!isRoot && (
        <Card>
          <CardHeader><CardTitle>Properties</CardTitle></CardHeader>
          <CardContent>
            {describe.isLoading ? (
              <div className="text-sm text-muted-foreground">Loading…</div>
            ) : (
              <PropertiesList properties={describe.data?.properties} />
            )}
          </CardContent>
        </Card>
      )}

      <PagedList
        title="Namespaces"
        icon={Folder}
        query={children}
        empty="No child namespaces."
        renderLink={(name) => (
          <Link
            to="/namespaces/$nsId"
            params={{ nsId: joinId([...segments, name]) }}
            className="font-mono text-primary hover:underline"
          >
            {name}
          </Link>
        )}
      />

      {!isRoot && (
        <PagedList
          title="Tables"
          icon={Table2}
          query={tables}
          empty="No tables in this namespace."
          renderLink={(name) => (
            <Link
              to="/tables/$tableId"
              params={{ tableId: joinId([...segments, name]) }}
              className="font-mono text-primary hover:underline"
            >
              {name}
            </Link>
          )}
        />
      )}

      <CreateNamespaceDialog parent={segments} open={createOpen} onOpenChange={setCreateOpen} />
      {!isRoot && <RegisterDialog namespace={segments} open={registerOpen} onOpenChange={setRegisterOpen} />}
    </div>
  );
}
