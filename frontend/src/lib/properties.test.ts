import { describe, expect, it } from "vitest";
import { buildProperties } from "./properties";

describe("buildProperties", () => {
  it("puts the owner under properties.owner and merges extra rows", () => {
    expect(buildProperties(" team ", [{ key: "tier", value: "gold" }, { key: "", value: "" }])).toEqual({
      ok: true,
      properties: { owner: "team", tier: "gold" },
    });
  });
  it("returns empty properties when nothing is set", () => {
    expect(buildProperties("", [])).toEqual({ ok: true, properties: {} });
  });
  it("rejects duplicate keys, including an explicit owner row", () => {
    expect(buildProperties("team", [{ key: "owner", value: "other" }]).ok).toBe(false);
    expect(buildProperties("", [{ key: "a", value: "1" }, { key: "a", value: "2" }]).ok).toBe(false);
  });
  it("rejects a value without a key", () => {
    expect(buildProperties("", [{ key: " ", value: "v" }]).ok).toBe(false);
  });
});
