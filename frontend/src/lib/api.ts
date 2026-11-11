import { idPathSegment } from "./ids";
import { z } from "zod";
import {
  columnStorageResponseSchema,
  createNamespaceResponseSchema,
  describeNamespaceResponseSchema,
  dropNamespaceResponseSchema,
  describeTableResponseSchema,
  errorResponseSchema,
  listNamespacesResponseSchema,
  listTableBranchesResponseSchema,
  listTableIndicesResponseSchema,
  listTableTagsResponseSchema,
  listTableVersionsResponseSchema,
  listTablesResponseSchema,
  registerTableResponseSchema,
  sqlResponseSchema,
  tableStatsResponseSchema,
  tableStorageResponseSchema,
  timelineResponseSchema,
  type TimelineResponse,
  type ColumnStorageResponse,
  type CreateNamespaceResponse,
  type DescribeNamespaceResponse,
  type DropNamespaceResponse,
  type DescribeTableResponse,
  type IndexContent,
  type ListNamespacesResponse,
  type ListTableBranchesResponse,
  type ListTableTagsResponse,
  type ListTableVersionsResponse,
  type ListTablesResponse,
  type RegisterTableResponse,
  type SqlResponse,
  type TableStatsResponse,
  type TableStorageResponse,
} from "./schemas";

const BASE = (import.meta.env.VITE_API_BASE as string | undefined) ?? "";

export class ApiError extends Error {
  status: number;
  /** Spec `ErrorCode`, when the body was a spec `ErrorResponse`. */
  code: number | undefined;
  constructor(status: number, message: string, code?: number) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.code = code;
  }
}

/** Build an `ApiError` from a non-2xx response body (spec `{error, code}`, or anything else). */
export function apiErrorFrom(status: number, statusText: string, body: string): ApiError {
  let message = `${status} ${statusText}`.trim();
  let code: number | undefined;
  try {
    const parsed = errorResponseSchema.safeParse(JSON.parse(body));
    if (parsed.success) {
      code = parsed.data.code;
      if (parsed.data.error) message = parsed.data.error;
    }
  } catch {
    // Non-JSON body: keep the status line.
  }
  return new ApiError(status, message, code);
}

interface Parser<T> {
  parse: (x: unknown) => T;
}

/** Decode a 2xx body. Malformed JSON or a body that does not match the expected model
 *  surfaces as an `ApiError` like any other failed call. */
export function decodeBody<T>(status: number, text: string, schema: Parser<T>): T {
  let json: unknown;
  try {
    json = text ? JSON.parse(text) : {};
  } catch {
    throw new ApiError(status, "The server returned malformed JSON.");
  }
  try {
    return schema.parse(json);
  } catch (e) {
    if (!(e instanceof z.ZodError)) throw e;
    const issue = e.issues[0];
    const where = issue?.path.map(String).join(".") || "body";
    throw new ApiError(status, `Unexpected response from the server (${where}: ${issue?.message ?? e.message}).`);
  }
}

async function request<T>(
  method: "GET" | "POST",
  path: string,
  schema: Parser<T>,
  body?: unknown,
  signal?: AbortSignal,
): Promise<T> {
  const init: RequestInit = { method, headers: { Accept: "application/json" }, signal };
  if (body !== undefined) {
    init.headers = { ...init.headers, "Content-Type": "application/json" };
    init.body = JSON.stringify(body);
  }
  const res = await fetch(BASE + path, init);
  const text = await res.text();
  if (!res.ok) throw apiErrorFrom(res.status, res.statusText, text);
  return decodeBody(res.status, text, schema);
}

/** Fetch every page of a spec list route, following `page_token` until it is absent. */
export async function followPages<P extends { page_token?: string | null }, T>(
  fetchPage: (token: string | null) => Promise<P>,
  items: (page: P) => readonly T[],
): Promise<T[]> {
  const out: T[] = [];
  let token: string | null = null;
  do {
    const page = await fetchPage(token);
    out.push(...items(page));
    token = page.page_token || null;
  } while (token);
  return out;
}

function pageQuery(pageToken?: string | null, limit?: number): string {
  const q = new URLSearchParams();
  if (pageToken) q.set("page_token", pageToken);
  if (limit !== undefined) q.set("limit", String(limit));
  const s = q.toString();
  return s ? `?${s}` : "";
}

// --- namespaces ---

export function listNamespaces(
  ns: readonly string[],
  pageToken?: string | null,
  limit?: number,
): Promise<ListNamespacesResponse> {
  return request(
    "GET",
    `/v1/namespace/${idPathSegment(ns)}/list${pageQuery(pageToken, limit)}`,
    listNamespacesResponseSchema,
  );
}

export function describeNamespace(ns: readonly string[]): Promise<DescribeNamespaceResponse> {
  return request("POST", `/v1/namespace/${idPathSegment(ns)}/describe`, describeNamespaceResponseSchema, {});
}

export function createNamespace(
  ns: readonly string[],
  properties?: Record<string, string>,
): Promise<CreateNamespaceResponse> {
  return request("POST", `/v1/namespace/${idPathSegment(ns)}/create`, createNamespaceResponseSchema, {
    ...(properties && Object.keys(properties).length ? { properties } : {}),
  });
}

export function dropNamespace(ns: readonly string[]): Promise<DropNamespaceResponse> {
  return request("POST", `/v1/namespace/${idPathSegment(ns)}/drop`, dropNamespaceResponseSchema, {});
}

export function listTables(
  ns: readonly string[],
  pageToken?: string | null,
  limit?: number,
): Promise<ListTablesResponse> {
  return request(
    "GET",
    `/v1/namespace/${idPathSegment(ns)}/table/list${pageQuery(pageToken, limit)}`,
    listTablesResponseSchema,
  );
}

/** One page of every table in the catalog; entries are full `$`-joined table ids. */
export function listAllTables(pageToken?: string | null, limit?: number): Promise<ListTablesResponse> {
  return request("GET", `/v1/table${pageQuery(pageToken, limit)}`, listTablesResponseSchema);
}

// --- tables ---

/** Cheap describe: catalog-recorded location and properties only; never opens the dataset. */
export function describeTableCheap(table: readonly string[]): Promise<DescribeTableResponse> {
  return request("POST", `/v1/table/${idPathSegment(table)}/describe`, describeTableResponseSchema, {});
}

/** Detailed describe: opens the dataset for version, schema, stats, and manifest metadata. */
export function describeTable(table: readonly string[]): Promise<DescribeTableResponse> {
  return request(
    "POST",
    `/v1/table/${idPathSegment(table)}/describe?load_detailed_metadata=true`,
    describeTableResponseSchema,
    {},
  );
}

/** One page of versions, newest first; `pageToken` is the last version of the previous page. */
export function listTableVersions(
  table: readonly string[],
  pageToken: string | null,
  limit: number,
): Promise<ListTableVersionsResponse> {
  const q = new URLSearchParams({ descending: "true", limit: String(limit) });
  if (pageToken) q.set("page_token", pageToken);
  return request("POST", `/v1/table/${idPathSegment(table)}/version/list?${q}`, listTableVersionsResponseSchema, {});
}

/** Every index on the current version, following `page_token` to the end. */
export function listAllTableIndices(table: readonly string[]): Promise<IndexContent[]> {
  return followPages(
    (token) =>
      request(
        "POST",
        `/v1/table/${idPathSegment(table)}/index/list`,
        listTableIndicesResponseSchema,
        token ? { page_token: token } : {},
      ),
    (page) => page.indexes,
  );
}

export function listTableTags(table: readonly string[]): Promise<ListTableTagsResponse> {
  return request("POST", `/v1/table/${idPathSegment(table)}/tags/list`, listTableTagsResponseSchema, {});
}

export function listTableBranches(table: readonly string[]): Promise<ListTableBranchesResponse> {
  return request("POST", `/v1/table/${idPathSegment(table)}/branches/list`, listTableBranchesResponseSchema, {});
}

/** Catalog route: bytes on disk per top-level column. Reads every fragment, so it is slow. */
export function getColumnStorage(table: readonly string[]): Promise<ColumnStorageResponse> {
  return request("GET", `/catalog/v1/table/${idPathSegment(table)}/column-storage`, columnStorageResponseSchema);
}

/** Catalog route: bytes on object storage under the table location, by file category, with
 *  the configured storage price. Lists the whole dataset prefix, so it is slow. */
export function getTableStorage(table: readonly string[]): Promise<TableStorageResponse> {
  return request("GET", `/catalog/v1/table/${idPathSegment(table)}/storage`, tableStorageResponseSchema);
}

/** Spec table stats (Lance `GetTableStats`): fragment row-count distribution and field-level
 *  data bytes. Reads every fragment's metadata, so it is slow on large tables. */
export function getTableStats(table: readonly string[]): Promise<TableStatsResponse> {
  return request("POST", `/v1/table/${idPathSegment(table)}/stats`, tableStatsResponseSchema, {});
}

/** Catalog route: one page of the table timeline, newest first; `pageToken` is the last
 *  version of the previous page. */
export function getTableTimeline(
  table: readonly string[],
  pageToken?: string | null,
  limit?: number,
): Promise<TimelineResponse> {
  return request(
    "GET",
    `/catalog/v1/table/${idPathSegment(table)}/timeline${pageQuery(pageToken, limit)}`,
    timelineResponseSchema,
  );
}

export function registerTable(
  table: readonly string[],
  location: string,
  properties?: Record<string, string>,
): Promise<RegisterTableResponse> {
  return request("POST", `/v1/table/${idPathSegment(table)}/register`, registerTableResponseSchema, {
    location,
    ...(properties && Object.keys(properties).length ? { properties } : {}),
  });
}

/** Catalog route: run one read-only SQL query. The plain table name reads `version` (the latest
 *  when omitted). Aborting `signal` only stops waiting; the server finishes or times out on its
 *  own. */
export function runSql(
  table: readonly string[],
  sql: string,
  version: number | null,
  signal?: AbortSignal,
): Promise<SqlResponse> {
  return request(
    "POST",
    `/catalog/v1/table/${idPathSegment(table)}/sql`,
    sqlResponseSchema,
    version == null ? { sql } : { sql, version },
    signal,
  );
}
