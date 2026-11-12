import { describe, expect, it } from "vitest";
import {
  formatBytes,
  formatCompact,
  formatCost,
  formatCount,
  formatDateTime,
  formatIsoDate,
  formatIsoTime,
  formatIsoTimestamp,
  formatPercent,
  formatRate,
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
  const d = new Date("2026-11-10T14:03:22Z");
  it("renders an absolute 24-hour time in UTC when preferred", () => {
    expect(formatDateTime(d, "utc")).toBe("Nov 10, 2026, 14:03:22 UTC");
  });
  it("renders local time with the local zone's offset", () => {
    const local = formatDateTime(d, "local");
    const hour = String(d.getHours()).padStart(2, "0");
    expect(local).toContain(`${hour}:03:22`);
    expect(formatDateTime(null, "utc")).toBe("—");
  });
});

describe("storage cost formatting", () => {
  it("prints the cost in cents and the rate at full precision", () => {
    expect(formatCost(255.2875, "USD")).toBe("$255.29");
    expect(formatCost(0, "USD")).toBe("$0.00");
    expect(formatRate(0.0133875, "USD")).toBe("$0.0133875 / GB-month");
    expect(formatRate(0.0065, "USD")).toBe("$0.0065 / GB-month");
  });
  it("falls back to the raw code for a non-ISO currency", () => {
    expect(formatCost(1.5, "credits")).toBe("1.50 credits");
  });
});

describe("ISO formatting", () => {
  // 2026-11-10T21:02:37.123Z, a Tuesday evening UTC.
  const d = new Date(Date.UTC(2026, 10, 10, 21, 2, 37, 123));

  it("formats in UTC with a Z offset", () => {
    expect(formatIsoDate(d, "utc")).toBe("2026-11-10");
    expect(formatIsoTime(d, "utc")).toBe("21:02:37");
    expect(formatIsoTimestamp(d, "utc")).toBe("2026-11-10T21:02:37.123Z");
  });

  it("formats local time with a fixed negative offset", () => {
    // Etc/GMT+8 is UTC-08:00 year-round.
    expect(formatIsoDate(d, "local", "Etc/GMT+8")).toBe("2026-11-10");
    expect(formatIsoTime(d, "local", "Etc/GMT+8")).toBe("13:02:37");
    expect(formatIsoTimestamp(d, "local", "Etc/GMT+8")).toBe("2026-11-10T13:02:37.123-08:00");
  });

  it("handles half-hour offsets and day rollover", () => {
    expect(formatIsoDate(d, "local", "Asia/Kolkata")).toBe("2026-11-11");
    expect(formatIsoTimestamp(d, "local", "Asia/Kolkata")).toBe("2026-11-11T02:32:37.123+05:30");
  });

  it("returns dash for null", () => {
    expect(formatIsoDate(null, "utc")).toBe("—");
    expect(formatIsoTime(null, "utc")).toBe("—");
    expect(formatIsoTimestamp(null, "local")).toBe("—");
  });
});
