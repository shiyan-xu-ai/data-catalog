import {
  auxSampleResponseSchema,
  describeNamespaceResponseSchema,
  errorResponseSchema,
  listNamespacesResponseSchema,
  tableEntrySchema,
  tableVersionSchema,
  ttlApplyResponseSchema,
  ttlAuditRecordSchema,
  ttlDryRunResponseSchema,
  type AuxSampleResponse,
  type DescribeNamespaceResponse,
  type ListNamespacesResponse,
  type TableEntry,
  type TableVersion,
  type TtlApplyResponse,
  type TtlAuditRecord,
  type TtlDryRunResponse,
} from "./schemas";

const BASE = (import.meta.env.VITE_API_BASE as string | undefined) ?? "";

export class ApiError extends Error {
  status: number;
  errorCode: number | undefined;
  constructor(status: number, message: string, errorCode?: number) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.errorCode = errorCode;
  }
}

async function parseError(res: Response): Promise<never> {
  let code: number | undefined;
  let message = `${res.status} ${res.statusText}`;
  try {
    const body = await res.json();
    const parsed = errorResponseSchema.safeParse(body);
    if (parsed.success) {
      code = parsed.data.error_code;
      message = parsed.data.message;
    }
  } catch {
    // non-JSON body — keep default message
  }
  throw new ApiError(res.status, message, code);
}

async function getJson<T>(path: string, schema: { parse: (x: unknown) => T }): Promise<T> {
  const res = await fetch(BASE + path, { headers: { Accept: "application/json" } });
  if (!res.ok) await parseError(res);
  const body = await res.json();
  return schema.parse(body);
}

async function sendJson<T>(
  method: "PUT" | "POST" | "DELETE",
  path: string,
  schema: { parse: (x: unknown) => T },
  body?: unknown,
): Promise<T> {
  const init: RequestInit = {
    method,
    headers: { Accept: "application/json" },
  };
  if (body !== undefined) {
    init.headers = { ...init.headers, "Content-Type": "application/json" };
    init.body = JSON.stringify(body);
  }
  const res = await fetch(BASE + path, init);
  if (!res.ok) await parseError(res);
  if (res.status === 204) return undefined as T;
  const text = await res.text();
  if (!text) return undefined as T;
  return schema.parse(JSON.parse(text));
}

// GET /v1/namespaces
export function listNamespaces(): Promise<ListNamespacesResponse> {
  return getJson("/v1/namespaces", listNamespacesResponseSchema);
}

// GET /v1/namespaces/:id  (id is dot-joined)
export function describeNamespace(id: string): Promise<DescribeNamespaceResponse> {
  return getJson(`/v1/namespaces/${encodeURIComponent(id)}`, describeNamespaceResponseSchema);
}

// GET /ext/v1/tables  (enriched listing; full merged detail)
export function listTables(): Promise<TableEntry[]> {
  return getJson("/ext/v1/tables", zArray(tableEntrySchema));
}

// GET /v1/table/:id
export function describeTable(id: string): Promise<TableEntry> {
  return getJson(`/v1/table/${encodeURIComponent(id)}`, tableEntrySchema);
}

// PUT /v1/table/:id  -- register/update owner+ttl_policy
export function declareTable(
  id: string,
  body: { owner?: string; ttl_policy?: { keep_last_n: number | null; max_age_days: number | null } },
): Promise<TableEntry> {
  return sendJson("PUT", `/v1/table/${encodeURIComponent(id)}`, tableEntrySchema, body);
}

// DELETE /v1/table/:id  -- deregister
export async function deregisterTable(id: string): Promise<void> {
  await sendJson("DELETE", `/v1/table/${encodeURIComponent(id)}`, zVoid);
}

// GET /ext/v1/tables/:id/versions/:vid
export function getVersion(id: string, vid: string): Promise<TableVersion> {
  return getJson(
    `/ext/v1/tables/${encodeURIComponent(id)}/versions/${encodeURIComponent(vid)}`,
    tableVersionSchema,
  );
}

// GET /ext/v1/tables/:id/versions/:vid/aux/sample?name=&limit=
// `name` may contain slashes (nested sidecar paths), so it must be URL-encoded.
export function sampleAux(
  id: string,
  vid: string,
  name: string,
  limit: number,
): Promise<AuxSampleResponse> {
  const q = new URLSearchParams({ name, limit: String(limit) });
  return getJson(
    `/ext/v1/tables/${encodeURIComponent(id)}/versions/${encodeURIComponent(vid)}/aux/sample?${q}`,
    auxSampleResponseSchema,
  );
}

// PUT /ext/v1/tables/:id/versions/:vid/protect
export function protectVersion(id: string, vid: string, protected_: boolean): Promise<TableVersion> {
  return sendJson(
    "PUT",
    `/ext/v1/tables/${encodeURIComponent(id)}/versions/${encodeURIComponent(vid)}/protect`,
    tableVersionSchema,
    { protected: protected_ },
  );
}

// GET /ext/v1/tables/:id/ttl/dryrun
export function ttlDryRun(id: string): Promise<TtlDryRunResponse> {
  return getJson(
    `/ext/v1/tables/${encodeURIComponent(id)}/ttl/dryrun`,
    ttlDryRunResponseSchema,
  );
}

// POST /ext/v1/tables/:id/ttl/apply
export function ttlApply(id: string): Promise<TtlApplyResponse> {
  return sendJson(
    "POST",
    `/ext/v1/tables/${encodeURIComponent(id)}/ttl/apply`,
    ttlApplyResponseSchema,
  );
}

// GET /ext/v1/tables/:id/ttl/audit
export function ttlAudit(id: string): Promise<TtlAuditRecord[]> {
  return getJson(
    `/ext/v1/tables/${encodeURIComponent(id)}/ttl/audit`,
    zArray(ttlAuditRecordSchema),
  );
}

// --- helpers ---

function zArray<T>(el: { parse: (x: unknown) => T }): { parse: (x: unknown) => T[] } {
  return {
    parse: (x: unknown) => {
      if (!Array.isArray(x)) throw new Error("expected array");
      return x.map((e) => el.parse(e));
    },
  };
}

const zVoid = {
  parse: (_x: unknown) => undefined,
};
