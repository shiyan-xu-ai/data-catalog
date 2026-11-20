import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiError, apiErrorFrom, decodeBody, followPages, runSql } from "./api";
import { ErrorCode, describeNamespaceResponseSchema, listTablesResponseSchema } from "./schemas";

afterEach(() => vi.unstubAllGlobals());

describe("runSql", () => {
  it("posts only the SQL to the global query route and returns the tables read", async () => {
    const body = {
      tables: [{ id: ["a", "b", "events"], version: 12 }],
      columns: [{ name: "id", type: "Int64" }],
      rows: [[1]],
      row_count: 1,
      truncated: false,
      elapsed_ms: 3,
    };
    const fetchMock = vi.fn(async () => new Response(JSON.stringify(body), { status: 200 }));
    vi.stubGlobal("fetch", fetchMock);
    const result = await runSql('SELECT id FROM "a"."b"."events"');
    const [url, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe("/catalog/v1/sql");
    expect(init.method).toBe("POST");
    expect(JSON.parse(init.body as string)).toEqual({ sql: 'SELECT id FROM "a"."b"."events"' });
    expect(result.tables).toEqual([{ id: ["a", "b", "events"], version: 12 }]);
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
