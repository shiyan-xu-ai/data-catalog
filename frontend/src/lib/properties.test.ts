import { describe, expect, it } from "vitest";
import { buildProperties } from "./properties";

describe("buildProperties", () => {
  it("puts set fields under their keys and merges extra rows", () => {
    expect(
      buildProperties({ owner: " team ", description: "Daily export" }, [
        { key: "tier", value: "gold" },
        { key: "", value: "" },
      ]),
    ).toEqual({
      ok: true,
      properties: { owner: "team", description: "Daily export", tier: "gold" },
    });
  });
  it("returns empty properties when nothing is set", () => {
    expect(buildProperties({ owner: "", description: " " }, [])).toEqual({ ok: true, properties: {} });
  });
  it("rejects duplicate keys, including a row repeating a set field", () => {
    expect(buildProperties({ owner: "team" }, [{ key: "owner", value: "other" }]).ok).toBe(false);
    expect(buildProperties({}, [{ key: "a", value: "1" }, { key: "a", value: "2" }]).ok).toBe(false);
  });
  it("rejects a value without a key", () => {
    expect(buildProperties({}, [{ key: " ", value: "v" }]).ok).toBe(false);
  });
  it("rejects keys under the reserved prefix only when one is given", () => {
    const rows = [{ key: "catalog.updated_at", value: "x" }];
    expect(buildProperties({}, rows, "catalog.").ok).toBe(false);
    expect(buildProperties({}, rows).ok).toBe(true);
  });
});
