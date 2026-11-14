import { describe, expect, it } from "vitest";
import { clipFileName } from "./media";
import { rangeHeader } from "./hevc-player";

describe("rangeHeader", () => {
  it("offsets chunks by the window origin and stops at the window's end", () => {
    expect(rangeHeader(0, 0, Infinity)).toBe(`bytes=0-${(2 << 20) - 1}`);
    expect(rangeHeader(1000, 0, 500)).toBe("bytes=1000-1499");
    expect(rangeHeader(1000, 2 << 20, (2 << 20) + 10)).toBe(`bytes=${1000 + (2 << 20)}-${1000 + (2 << 20) + 9}`);
  });
});

describe("clipFileName", () => {
  it("puts the byte range before the object's extension", () => {
    expect(clipFileName("https://h/a/pack_00000.h265?X-Amz-Signature=1", { start: 10, length: 5 })).toBe("pack_00000_10-15.h265");
    expect(clipFileName("s3://b/noext", { start: 0, length: 3 })).toBe("noext_0-3");
  });
});
