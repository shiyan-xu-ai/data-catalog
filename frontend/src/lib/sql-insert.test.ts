import { describe, expect, it, vi } from "vitest";
import { insertTablePath, registerSqlInsert } from "./sql-insert";

describe("insertTablePath", () => {
  it("does nothing without a registered handler", () => {
    expect(() => insertTablePath(["a", "t"])).not.toThrow();
  });

  it("hands the quoted full path to the handler", () => {
    const handler = vi.fn();
    const unregister = registerSqlInsert(handler);
    insertTablePath(["a", 'b"c', "events"]);
    expect(handler).toHaveBeenCalledWith('"a"."b""c"."events"');
    unregister();
    insertTablePath(["a", "t"]);
    expect(handler).toHaveBeenCalledTimes(1);
  });

  it("keeps the newest handler when an older registration is cleaned up late", () => {
    const first = vi.fn();
    const second = vi.fn();
    const unregisterFirst = registerSqlInsert(first);
    const unregisterSecond = registerSqlInsert(second);
    unregisterFirst();
    insertTablePath(["a", "t"]);
    expect(second).toHaveBeenCalledOnce();
    expect(first).not.toHaveBeenCalled();
    unregisterSecond();
  });
});
