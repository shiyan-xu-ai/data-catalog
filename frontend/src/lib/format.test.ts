import { describe, expect, it } from "vitest";
import {
  formatBytes,
  formatCompact,
  formatCount,
  formatDateTime,
  formatPercent,
  formatRelative,
  parseTime,
} from "./format";

describe("formatCount", () => {
  it("returns dash for null/undefined and groups digits", () => {
    expect(formatCount(null)).toBe("—");
    expect(formatCount(undefined)).toBe("—");
    expect(formatCount(0)).toBe("0");
    expect(formatCount(8047929)).toBe("8,047,929");
  });
});

describe("formatCompact", () => {
  it("abbreviates large counts", () => {
    expect(formatCompact(8047929)).toBe("8.05M");
    expect(formatCompact(950)).toBe("950");
  });
});

describe("formatBytes", () => {
  it("uses decimal units with three significant digits", () => {
    expect(formatBytes(null)).toBe("—");
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(999)).toBe("999 B");
    expect(formatBytes(1500)).toBe("1.50 KB");
    expect(formatBytes(990_123_456)).toBe("990 MB");
    expect(formatBytes(12_345_678_901)).toBe("12.3 GB");
  });
});

describe("formatPercent", () => {
  it("handles edges without rounding to misleading values", () => {
    expect(formatPercent(null)).toBe("—");
    expect(formatPercent(0)).toBe("0%");
    expect(formatPercent(1)).toBe("100%");
    expect(formatPercent(0.0001)).toBe("<0.1%");
    expect(formatPercent(0.99999)).toBe(">99.9%");
    expect(formatPercent(0.053)).toBe("5.3%");
  });
});

describe("parseTime", () => {
  it("parses RFC 3339 and epoch millis, rejecting garbage", () => {
    expect(parseTime("2026-11-10T02:35:00Z")?.getTime()).toBe(Date.UTC(2026, 10, 10, 2, 35));
    expect(parseTime(0)?.getTime()).toBe(0);
    expect(parseTime("nope")).toBeNull();
    expect(parseTime(undefined)).toBeNull();
    expect(parseTime("")).toBeNull();
  });
});

describe("formatRelative", () => {
  const now = new Date("2026-11-10T12:00:00Z");
  it("formats past and future offsets", () => {
    expect(formatRelative(new Date("2026-11-10T11:59:50Z"), now)).toBe("just now");
    expect(formatRelative(new Date("2026-11-10T09:00:00Z"), now)).toBe("3 hours ago");
    expect(formatRelative(new Date("2026-11-09T12:00:00Z"), now)).toBe("yesterday");
    expect(formatRelative(new Date("2026-11-12T12:00:00Z"), now)).toBe("in 2 days");
    expect(formatRelative(new Date("2024-11-10T12:00:00Z"), now)).toBe("2 years ago");
    expect(formatRelative(null, now)).toBe("—");
  });
});

describe("formatDateTime", () => {
  it("renders an absolute 24-hour time in the given zone", () => {
    expect(formatDateTime(new Date("2026-11-10T14:03:22Z"), "UTC")).toBe("Nov 10, 2026, 14:03:22 UTC");
  });
});
