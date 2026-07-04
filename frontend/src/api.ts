// Thin fetch wrapper around the catalog-api. Base URL is same-origin by default
// (dev proxy or prod static host); override with VITE_API_BASE for cross-origin.
import type {
  TableEntry,
  TableVersion,
  TtlApplyResponse,
  TtlAuditRecord,
  TtlDryRunResponse,
} from "./types";

const BASE = (import.meta.env.VITE_API_BASE as string | undefined) ?? "";

async function getJson<T>(path: string): Promise<T> {
  const res = await fetch(BASE + path, { headers: { Accept: "application/json" } });
  if (!res.ok) {
    throw new Error(`${res.status} ${res.statusText} on ${path}`);
  }
  return (await res.json()) as T;
}

async function postJson<T>(path: string): Promise<T> {
  const res = await fetch(BASE + path, {
    method: "POST",
    headers: { Accept: "application/json" },
  });
  if (!res.ok) {
    const body = await res.text().catch(() => "");
    throw new Error(`${res.status} ${res.statusText} on ${path}: ${body}`);
  }
  return (await res.json()) as T;
}

export function listTables(): Promise<TableEntry[]> {
  return getJson<TableEntry[]>("/ext/v1/tables");
}

export function describeTable(id: string): Promise<TableEntry> {
  return getJson<TableEntry>(`/v1/table/${encodeURIComponent(id)}`);
}

export function getVersion(id: string, vid: string): Promise<TableVersion> {
  return getJson<TableVersion>(
    `/ext/v1/tables/${encodeURIComponent(id)}/versions/${encodeURIComponent(vid)}`,
  );
}

export function ttlDryRun(id: string): Promise<TtlDryRunResponse> {
  return getJson<TtlDryRunResponse>(
    `/ext/v1/tables/${encodeURIComponent(id)}/ttl/dryrun`,
  );
}

export function ttlApply(id: string): Promise<TtlApplyResponse> {
  return postJson<TtlApplyResponse>(
    `/ext/v1/tables/${encodeURIComponent(id)}/ttl/apply`,
  );
}

export function ttlAudit(id: string): Promise<TtlAuditRecord[]> {
  return getJson<TtlAuditRecord[]>(
    `/ext/v1/tables/${encodeURIComponent(id)}/ttl/audit`,
  );
}
