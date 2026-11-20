import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiError, apiErrorFrom, cancelSql, decodeBody, followPages, runSql } from "./api";
import { ErrorCode, describeNamespaceResponseSchema, listTablesResponseSchema } from "./schemas";

afterEach(() => vi.unstubAllGlobals());

describe("runSql", () => {
  it("posts the SQL and query id to the global query route and returns the result", async () => {
    const body = {
      tables: [{ id: ["a", "b", "events"], version: 12 }],
      columns: [{ name: "id", type: "Int64" }],
      rows: [[1]],
      row_count: 1,
      truncated: true,
      truncated_by: "rows",
      cached: true,
      elapsed_ms: 3,
    };
    const fetchMock = vi.fn(async () => new Response(JSON.stringify(body), { status: 200 }));
    vi.stubGlobal("fetch", fetchMock);
    const result = await runSql('SELECT id FROM "a"."b"."events"', "q-1");
    const [url, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe("/catalog/v1/sql");
    expect(init.method).toBe("POST");
    expect(JSON.parse(init.body as string)).toEqual({ sql: 'SELECT id FROM "a"."b"."events"', query_id: "q-1" });
    expect(result.tables).toEqual([{ id: ["a", "b", "events"], version: 12 }]);
    expect(result.cached).toBe(true);
    expect(result.truncated_by).toBe("rows");
  });
  it("keeps the server's rejection message", async () => {
    const message = "you already have 2 queries running; wait for one or cancel it";
    vi.stubGlobal("fetch", async () => new Response(JSON.stringify({ error: message, code: 21 }), { status: 429 }));
    const e = await runSql("SELECT 1", "q-1").catch((x: unknown) => x);
    expect(e).toBeInstanceOf(ApiError);
    expect((e as ApiError).code).toBe(ErrorCode.Throttling);
    expect((e as ApiError).message).toBe(message);
  });
});

describe("cancelSql", () => {
  it("posts to the query's cancel route with keepalive, so it survives page unload", async () => {
    const fetchMock = vi.fn(async () => new Response(null, { status: 204 }));
    vi.stubGlobal("fetch", fetchMock);
    expect(await cancelSql("q 1/x")).toBe(true);
    const [url, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe("/catalog/v1/sql/q%201%2Fx/cancel");
    expect(init.method).toBe("POST");
    expect(init.keepalive).toBe(true);
  });
  it("reports a 404 as unconfirmed rather than failing", async () => {
    vi.stubGlobal("fetch", async () => new Response(JSON.stringify({ error: "no such query", code: 4 }), { status: 404 }));
    expect(await cancelSql("q-1")).toBe(false);
  });
  it("raises other failures", async () => {
    vi.stubGlobal("fetch", async () => new Response("", { status: 500, statusText: "Internal Server Error" }));
    await expect(cancelSql("q-1")).rejects.toBeInstanceOf(ApiError);
  });
});

describe("apiErrorFrom", () => {
  it("uses the spec error message and code", () => {
    const e = apiErrorFrom(409, "Conflict", JSON.stringify({ error: "Namespace not empty", code: 3 }));
    expect(e.status).toBe(409);
    expect(e.code).toBe(ErrorCode.NamespaceNotEmpty);
    expect(e.message).toBe("Namespace not empty");
  });
  it("falls back to the status line for non-spec bodies", () => {
    const e = apiErrorFrom(502, "Bad Gateway", "<html>");
    expect(e.code).toBeUndefined();
    expect(e.message).toBe("502 Bad Gateway");
  });
});

describe("decodeBody", () => {
  const schema = listTablesResponseSchema;
  it("parses a valid body and treats an empty body as {}", () => {
    expect(decodeBody(200, '{"tables":["t"]}', schema).tables).toEqual(["t"]);
    expect(decodeBody(200, "", describeNamespaceResponseSchema)).toEqual({});
  });
  it("raises ApiError for malformed JSON and for shape mismatches", () => {
    expect(() => decodeBody(200, "<html>", schema)).toThrow(ApiError);
    expect(() => decodeBody(200, '{"tables":[1]}', schema)).toThrow(ApiError);
    expect(() => decodeBody(200, '{"tables":[1]}', schema)).toThrow(/tables\.0/);
  });
});

describe("followPages", () => {
  it("follows page_token until it is absent and concatenates items", async () => {
    const pages: Record<string, { items: number[]; page_token?: string | null }> = {
      start: { items: [1, 2], page_token: "b" },
      b: { items: [3], page_token: "" },
    };
    const seen: (string | null)[] = [];
    const all = await followPages(
      async (token) => {
        seen.push(token);
        return pages[token ?? "start"];
      },
      (p) => p.items,
    );
    expect(all).toEqual([1, 2, 3]);
    expect(seen).toEqual([null, "b"]);
  });
});
