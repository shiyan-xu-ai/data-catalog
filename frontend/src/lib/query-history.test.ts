import { afterEach, describe, expect, it, vi } from "vitest";
import { MAX_HISTORY, addHistory, loadHistory, saveHistory, type HistoryEntry } from "./query-history";

afterEach(() => vi.unstubAllGlobals());

function stubStorage(initial: Record<string, string> = {}) {
  const data = { ...initial };
  vi.stubGlobal("localStorage", {
    getItem: (k: string) => data[k] ?? null,
    setItem: (k: string, v: string) => {
      data[k] = v;
    },
  });
  return data;
}

describe("addHistory", () => {
  it("puts the newest run first", () => {
    const entries = addHistory(addHistory([], "SELECT 1", 1), "SELECT 2", 2);
    expect(entries).toEqual([
      { sql: "SELECT 2", at: 2 },
      { sql: "SELECT 1", at: 1 },
    ]);
  });

  it("moves a repeated text to the top with the new time instead of adding a row", () => {
    let entries = addHistory([], "SELECT 1", 1);
    entries = addHistory(entries, "SELECT 2", 2);
    entries = addHistory(entries, "SELECT 1", 3);
    expect(entries).toEqual([
      { sql: "SELECT 1", at: 3 },
      { sql: "SELECT 2", at: 2 },
    ]);
  });

  it("drops the oldest entry when the 51st is added", () => {
    let entries: HistoryEntry[] = [];
    for (let i = 1; i <= MAX_HISTORY + 1; i++) entries = addHistory(entries, `SELECT ${i}`, i);
    expect(entries).toHaveLength(50);
    expect(entries[0].sql).toBe("SELECT 51");
    expect(entries[49].sql).toBe("SELECT 2");
  });
});

describe("loadHistory and saveHistory", () => {
  it("round-trips through localStorage", () => {
    stubStorage();
    const entries = [{ sql: "SELECT 1", at: 5 }];
    saveHistory(entries);
    expect(loadHistory()).toEqual(entries);
  });

  it("loads empty when nothing is stored or the stored data is unreadable", () => {
    stubStorage();
    expect(loadHistory()).toEqual([]);
    stubStorage({ "query-history": "{not json" });
    expect(loadHistory()).toEqual([]);
    stubStorage({ "query-history": '{"sql":"SELECT 1","at":1}' });
    expect(loadHistory()).toEqual([]);
  });

  it("skips malformed entries", () => {
    stubStorage({ "query-history": JSON.stringify([{ sql: "SELECT 1", at: 1 }, { sql: 3 }, null]) });
    expect(loadHistory()).toEqual([{ sql: "SELECT 1", at: 1 }]);
  });

  it("tolerates storage that is missing or throws", () => {
    vi.stubGlobal("localStorage", {
      getItem: () => {
        throw new Error("denied");
      },
      setItem: () => {
        throw new Error("denied");
      },
    });
    expect(loadHistory()).toEqual([]);
    expect(() => saveHistory([{ sql: "SELECT 1", at: 1 }])).not.toThrow();
  });
});
