import { describe, expect, it } from "vitest";
import { displayName, idPathSegment, joinId, segmentError, splitId } from "./ids";

describe("joinId / splitId", () => {
  it("encodes the root namespace as $", () => {
    expect(joinId([])).toBe("$");
    expect(splitId("$")).toEqual([]);
    expect(splitId("")).toEqual([]);
  });
  it("round-trips nested segments", () => {
    const segs = ["a", "b.c", "tbl"];
    expect(joinId(segs)).toBe("a$b.c$tbl");
    expect(splitId(joinId(segs))).toEqual(segs);
  });
  it("rejects segments that would corrupt the id", () => {
    expect(() => joinId(["a$b"])).toThrow();
    expect(() => joinId(["a", ""])).toThrow();
  });
});

describe("idPathSegment", () => {
  it("URL-encodes the joined id as one path segment", () => {
    expect(idPathSegment([])).toBe("%24");
    expect(idPathSegment(["a b", "x/y"])).toBe("a%20b%24x%2Fy");
  });
});

describe("segmentError", () => {
  it("accepts plain names and rejects empty, $, and padded names", () => {
    expect(segmentError("ok_name")).toBeNull();
    expect(segmentError("")).not.toBeNull();
    expect(segmentError("a$b")).not.toBeNull();
    expect(segmentError(" a")).not.toBeNull();
  });
});

describe("displayName", () => {
  it("joins segments with dots", () => {
    expect(displayName(["a", "b", "c"])).toBe("a.b.c");
    expect(displayName([])).toBe("");
  });
});
