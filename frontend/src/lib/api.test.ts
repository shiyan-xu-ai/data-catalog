import { describe, expect, it } from "vitest";
import { apiErrorFrom } from "./api";
import { ErrorCode } from "./schemas";

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
