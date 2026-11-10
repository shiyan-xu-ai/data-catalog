import { describe, expect, it } from "vitest";
import { buildSearchIndex, enumerateNamespaces, searchCatalog } from "./search";

const INDEX = buildSearchIndex(
  [
    ["scenario", "export", "daily_v2"],
    ["scenario", "daily"],
    ["logs", "raw"],
  ],
  [["empty_ns"], ["scenario"]],
);

describe("buildSearchIndex", () => {
  it("adds every table ancestor as a namespace once", () => {
    const ns = INDEX.filter((e) => e.kind === "namespace").map((e) => e.segments.join("."));
    expect(ns.sort()).toEqual(["empty_ns", "logs", "scenario", "scenario.export"]);
    expect(INDEX.filter((e) => e.kind === "table")).toHaveLength(3);
  });
});

describe("searchCatalog", () => {
  it("ranks exact, prefix, substring, then path matches", () => {
    const names = (q: string) => searchCatalog(INDEX, q, 10).map((e) => e.segments.join("."));
    expect(names("daily")).toEqual(["scenario.daily", "scenario.export.daily_v2"]);
    expect(names("EXPORT")).toEqual(["scenario.export", "scenario.export.daily_v2"]);
    expect(names("scenario.export.d")).toEqual(["scenario.export.daily_v2"]);
    expect(names("  ")).toEqual([]);
  });
  it("respects the limit", () => {
    expect(searchCatalog(INDEX, "a", 2)).toHaveLength(2);
  });
});

describe("enumerateNamespaces", () => {
  const tree: Record<string, string[]> = { "": ["a", "b"], a: ["x", "y"], "a.x": ["deep"], b: [] };
  const list = async (parent: string[]) => tree[parent.join(".")] ?? [];

  it("walks every level, including empty namespaces", async () => {
    const found = await enumerateNamespaces(list, { max: 100, concurrency: 2 });
    expect(found.map((s) => s.join(".")).sort()).toEqual(["a", "a.x", "a.x.deep", "a.y", "b"]);
  });
  it("stops at the cap", async () => {
    expect(await enumerateNamespaces(list, { max: 3, concurrency: 1 })).toHaveLength(3);
  });
});
