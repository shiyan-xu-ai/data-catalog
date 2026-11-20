import { describe, expect, it } from "vitest";
import { initialTabs, tabTitle, tabsReducer, type TabsState } from "./query-tabs";

const ids = (s: TabsState) => s.tabs.map((t) => t.id);

describe("query tabs", () => {
  it("starts with one tab holding the given text", () => {
    expect(initialTabs()).toEqual({ tabs: [{ id: 1, text: "" }], activeId: 1, nextId: 2 });
    expect(initialTabs("SELECT 1").tabs[0].text).toBe("SELECT 1");
  });

  it("opens a pre-filled tab and activates it, leaving other tabs' text alone", () => {
    let s = initialTabs("first");
    s = tabsReducer(s, { type: "open", text: "second" });
    s = tabsReducer(s, { type: "open", text: 'SELECT * FROM "a"."b"."t" LIMIT 20' });
    expect(s.tabs.map((t) => t.text)).toEqual(["first", "second", 'SELECT * FROM "a"."b"."t" LIMIT 20']);
    expect(s.activeId).toBe(3);
  });

  it("edits only the named tab", () => {
    let s = tabsReducer(initialTabs("one"), { type: "open", text: "two" });
    s = tabsReducer(s, { type: "setText", id: 1, text: "edited" });
    expect(s.tabs.map((t) => t.text)).toEqual(["edited", "two"]);
  });

  it("closing the active tab activates the neighbor that takes its place, or the previous for the last", () => {
    let s = initialTabs();
    s = tabsReducer(s, { type: "open" });
    s = tabsReducer(s, { type: "open" });
    s = tabsReducer(s, { type: "activate", id: 2 });
    s = tabsReducer(s, { type: "close", id: 2 });
    expect(ids(s)).toEqual([1, 3]);
    expect(s.activeId).toBe(3);
    s = tabsReducer(s, { type: "close", id: 3 });
    expect(s.activeId).toBe(1);
  });

  it("closing an inactive tab keeps the active tab", () => {
    let s = tabsReducer(initialTabs(), { type: "open" });
    s = tabsReducer(s, { type: "close", id: 1 });
    expect(ids(s)).toEqual([2]);
    expect(s.activeId).toBe(2);
  });

  it("closing the last remaining tab opens a fresh one", () => {
    const s = tabsReducer(initialTabs("draft"), { type: "close", id: 1 });
    expect(s.tabs).toEqual([{ id: 2, text: "" }]);
    expect(s.activeId).toBe(2);
  });

  it("never reuses a tab number, and titles carry it", () => {
    let s = initialTabs();
    s = tabsReducer(s, { type: "open" });
    s = tabsReducer(s, { type: "close", id: 2 });
    s = tabsReducer(s, { type: "open" });
    expect(ids(s)).toEqual([1, 3]);
    expect(s.tabs.map(tabTitle)).toEqual(["Query 1", "Query 3"]);
  });

  it("ignores unknown tabs", () => {
    const s = tabsReducer(initialTabs(), { type: "open" });
    expect(tabsReducer(s, { type: "close", id: 9 })).toBe(s);
    expect(tabsReducer(s, { type: "activate", id: 9 })).toBe(s);
  });
});
