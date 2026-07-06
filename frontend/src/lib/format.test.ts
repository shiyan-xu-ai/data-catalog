import { describe, expect, it } from "vitest";
import { formatBytes, formatTime, nsToString, shapeLabel, ttlPolicyString } from "./format";
import { ttlPolicySchema } from "./schemas";

describe("formatTime", () => {
  it("renders the instant in UTC when mode is utc", () => {
    const out = formatTime("2026-07-05T12:00:00Z", "utc");
    expect(out).toContain("12:00");
    expect(out).toContain("GMT");
  });
  it("returns dash for missing input", () => {
    expect(formatTime(null, "utc")).toBe("—");
  });
});

describe("formatBytes", () => {
  it("returns dash for null/0", () => {
    expect(formatBytes(null)).toBe("—");
    expect(formatBytes(0)).toBe("—");
  });
  it("formats bytes under 1 KiB as B", () => {
    expect(formatBytes(512)).toBe("512 B");
  });
  it("formats KiB and above with one decimal when small", () => {
    expect(formatBytes(1536)).toBe("1.5 KiB");
    expect(formatBytes(1048576)).toBe("1 MiB");
  });
  it("rounds GiB and above", () => {
    expect(formatBytes(1073741824 * 5)).toBe("5 GiB");
  });
});

describe("nsToString", () => {
  it("joins segments with dots", () => {
    expect(nsToString(["a", "b", "c"])).toBe("a.b.c");
    expect(nsToString([])).toBe("");
  });
});

describe("ttlPolicyString", () => {
  it("returns none for null", () => {
    expect(ttlPolicyString(null)).toBe("none");
  });
  it("returns none for empty policy", () => {
    const p = ttlPolicySchema.parse({ keep_last_n: null, max_age_days: null });
    expect(ttlPolicyString(p)).toBe("none");
  });
  it("combines both fields", () => {
    const p = ttlPolicySchema.parse({ keep_last_n: 5, max_age_days: 30 });
    expect(ttlPolicyString(p)).toBe("keep last 5, max age 30d");
  });
  it("shows only set field", () => {
    const p = ttlPolicySchema.parse({ keep_last_n: 10, max_age_days: null });
    expect(ttlPolicyString(p)).toBe("keep last 10");
  });
});

describe("shapeLabel", () => {
  it("maps known shapes", () => {
    expect(shapeLabel("full")).toBe("full");
    expect(shapeLabel("lance_only_partial")).toBe("lance partial");
    expect(shapeLabel("seg_only")).toBe("segments only");
  });
  it("passes through unknown", () => {
    expect(shapeLabel("weird")).toBe("weird");
  });
});
