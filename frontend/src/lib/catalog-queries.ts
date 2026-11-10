import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import {
  describeTable,
  describeTableCheap,
  followPages,
  getColumnStorage,
  listAllTableIndices,
  listAllTables,
  listNamespaces,
  listTableBranches,
  listTableTags,
  listTableVersions,
  listTables,
} from "./api";
import { joinId, splitId } from "./ids";
import { createLimiter } from "./limit";
import { enumerateNamespaces } from "./search";

// Query keys. Table keys share the `["table", id]` prefix so deregistration can drop them all.
export const keys = {
  namespace: (id: string) => ["namespace", id] as const,
  namespaces: (id: string) => ["namespaces", id] as const,
  tables: (id: string) => ["tables", id] as const,
  allTables: ["all-tables"] as const,
  allNamespaces: ["all-namespaces"] as const,
  table: (id: string) => ["table", id] as const,
  tableCheap: (id: string) => ["table", id, "cheap"] as const,
  tableDetail: (id: string) => ["table", id, "detail"] as const,
  tableVersions: (id: string) => ["table", id, "versions"] as const,
  tableIndices: (id: string) => ["table", id, "indices"] as const,
  tableTags: (id: string) => ["table", id, "tags"] as const,
  tableBranches: (id: string) => ["table", id, "branches"] as const,
  tableColumns: (id: string) => ["table", id, "columns"] as const,
};

const PAGE_SIZE = 100;

export interface PagedNames {
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
  fetchPage: (token: string | null) => Promise<{ names: string[]; page_token?: string | null }>,
  enabled: boolean,
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

export function useChildNamespaces(segments: readonly string[], enabled = true): PagedNames {
  return usePagedNames(
    keys.namespaces(joinId(segments)),
    async (token) => {
      const r = await listNamespaces(segments, token, PAGE_SIZE);
      return { names: r.namespaces, page_token: r.page_token };
    },
    enabled,
  );
}

/** Tables directly in a namespace. The root holds no tables, so it is never queried. */
export function useNamespaceTables(segments: readonly string[], enabled = true): PagedNames {
  return usePagedNames(
    keys.tables(joinId(segments)),
    async (token) => {
      const r = await listTables(segments, token, PAGE_SIZE);
      return { names: r.tables, page_token: r.page_token };
    },
    enabled && segments.length > 0,
  );
}

/** Every table id in the catalog as segment arrays, for global search. */
export function useAllTableIds(enabled: boolean) {
  return useQuery({
    queryKey: keys.allTables,
    queryFn: async () =>
      (await followPages((token) => listAllTables(token, 1000), (page) => page.tables)).map(splitId),
    enabled,
    staleTime: 60_000,
  });
}

// Search enumerates the namespace tree so empty namespaces are findable; bounded so a huge
// catalog cannot turn one search into thousands of requests.
const MAX_SEARCH_NAMESPACES = 5000;
const NAMESPACE_LIST_CONCURRENCY = 4;

/** Every namespace (up to a cap) as segment arrays, for global search. */
export function useAllNamespaces(enabled: boolean) {
  return useQuery({
    queryKey: keys.allNamespaces,
    queryFn: () =>
      enumerateNamespaces(
        (parent) => followPages((token) => listNamespaces(parent, token, PAGE_SIZE), (page) => page.namespaces),
        { max: MAX_SEARCH_NAMESPACES, concurrency: NAMESPACE_LIST_CONCURRENCY },
      ),
    enabled,
    staleTime: 60_000,
  });
}

// Cheap describes for list rows run through one shared limiter so a long table list never
// floods the server (KTD7).
const describeLimit = createLimiter(6);

export function useTableCheap(segments: readonly string[], enabled = true) {
  return useQuery({
    queryKey: keys.tableCheap(joinId(segments)),
    queryFn: () => describeLimit(() => describeTableCheap(segments)),
    enabled,
  });
}

export function useTableDetail(segments: readonly string[]) {
  return useQuery({ queryKey: keys.tableDetail(joinId(segments)), queryFn: () => describeTable(segments) });
}

/** All versions, newest first, following `page_token` to the end. */
export function useTableVersions(segments: readonly string[]) {
  return useQuery({
    queryKey: keys.tableVersions(joinId(segments)),
    queryFn: async () => {
      const all = await followPages(
        (token) => listTableVersions(segments, token, PAGE_SIZE),
        (page) => page.versions,
      );
      return all.sort((a, b) => b.version - a.version);
    },
  });
}

export function useTableIndices(segments: readonly string[]) {
  return useQuery({ queryKey: keys.tableIndices(joinId(segments)), queryFn: () => listAllTableIndices(segments) });
}

export function useTableTags(segments: readonly string[]) {
  return useQuery({ queryKey: keys.tableTags(joinId(segments)), queryFn: () => listTableTags(segments) });
}

export function useTableBranches(segments: readonly string[]) {
  return useQuery({ queryKey: keys.tableBranches(joinId(segments)), queryFn: () => listTableBranches(segments) });
}

/** Per-column bytes; slow, so callers enable it only after the rest of the page loaded. */
export function useColumnStorage(segments: readonly string[], enabled: boolean) {
  return useQuery({
    queryKey: keys.tableColumns(joinId(segments)),
    queryFn: () => getColumnStorage(segments),
    enabled,
  });
}
