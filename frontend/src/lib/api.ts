import { idPathSegment } from "./ids";
import {
  createNamespaceResponseSchema,
  deregisterTableResponseSchema,
  describeNamespaceResponseSchema,
  dropNamespaceResponseSchema,
  describeTableResponseSchema,
  errorResponseSchema,
  listNamespacesResponseSchema,
  listTablesResponseSchema,
  registerTableResponseSchema,
  type CreateNamespaceResponse,
  type DeregisterTableResponse,
  type DescribeNamespaceResponse,
  type DropNamespaceResponse,
  type DescribeTableResponse,
  type ListNamespacesResponse,
  type ListTablesResponse,
  type RegisterTableResponse,
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

async function request<T>(
  method: "GET" | "POST",
  path: string,
  schema: Parser<T>,
  body?: unknown,
): Promise<T> {
  const init: RequestInit = { method, headers: { Accept: "application/json" } };
  if (body !== undefined) {
    init.headers = { ...init.headers, "Content-Type": "application/json" };
    init.body = JSON.stringify(body);
  }
  const res = await fetch(BASE + path, init);
  const text = await res.text();
  if (!res.ok) throw apiErrorFrom(res.status, res.statusText, text);
  return schema.parse(text ? JSON.parse(text) : {});
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

// --- tables ---

/** Detailed describe: opens the dataset for version, schema, and stats. */
export function describeTable(table: readonly string[]): Promise<DescribeTableResponse> {
  return request(
    "POST",
    `/v1/table/${idPathSegment(table)}/describe?load_detailed_metadata=true`,
    describeTableResponseSchema,
    {},
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

export function deregisterTable(table: readonly string[]): Promise<DeregisterTableResponse> {
  return request("POST", `/v1/table/${idPathSegment(table)}/deregister`, deregisterTableResponseSchema, {});
}
