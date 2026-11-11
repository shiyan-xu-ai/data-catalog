import { describe, expect, it } from "vitest";
import { ApiError, apiErrorFrom, decodeBody, followPages } from "./api";
import { ErrorCode, describeNamespaceResponseSchema, listTablesResponseSchema } from "./schemas";

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
