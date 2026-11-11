const DASH = "—";

export function formatCount(n: number | null | undefined): string {
  if (n == null) return DASH;
  return n.toLocaleString("en-US");
}

/** `n` with a singular or plural noun, e.g. "1 deletion file", "4,519 data files". */
export function formatCountOf(n: number, singular: string, plural = `${singular}s`): string {
  return `${formatCount(n)} ${n === 1 ? singular : plural}`;
}

/** Short count for chart axes: `8.05M`, `990K`. */
export function formatCompact(n: number | null | undefined): string {
  if (n == null) return DASH;
  return new Intl.NumberFormat("en-US", { notation: "compact", maximumFractionDigits: 2 }).format(n);
}

const BYTE_UNITS = ["B", "KB", "MB", "GB", "TB", "PB"];

/** Decimal (SI) byte size with three significant digits: `990 MB`, `1.25 GB`. */
export function formatBytes(n: number | null | undefined): string {
  if (n == null) return DASH;
  if (n < 1000) return `${n} B`;
  let unit = 0;
  let v = n;
  while (v >= 1000 && unit < BYTE_UNITS.length - 1) {
    v /= 1000;
    unit++;
  }
  const digits = v >= 100 ? 0 : v >= 10 ? 1 : 2;
  return `${v.toFixed(digits)} ${BYTE_UNITS[unit]}`;
}

export function formatPercent(fraction: number | null | undefined): string {
  if (fraction == null || !Number.isFinite(fraction)) return DASH;
  const pct = fraction * 100;
  if (pct > 0 && pct < 0.1) return "<0.1%";
  if (pct < 100 && pct > 99.9) return ">99.9%";
  return `${pct.toFixed(pct >= 10 || pct === 0 ? 0 : 1)}%`;
}

/** Parse an RFC 3339 string or epoch milliseconds; null when absent or invalid. */
export function parseTime(value: string | number | null | undefined): Date | null {
  if (value == null || value === "") return null;
  const d = new Date(value);
  return Number.isNaN(d.getTime()) ? null : d;
}

const RELATIVE_STEPS: [Intl.RelativeTimeFormatUnit, number][] = [
  ["second", 60],
  ["minute", 60],
  ["hour", 24],
  ["day", 30],
  ["month", 12],
  ["year", Number.POSITIVE_INFINITY],
];

/** `3 hours ago`, `in 2 days`, `just now`. */
export function formatRelative(date: Date | null, now: Date = new Date()): string {
  if (!date) return DASH;
  let delta = (date.getTime() - now.getTime()) / 1000;
  if (Math.abs(delta) < 45) return "just now";
  const rtf = new Intl.RelativeTimeFormat("en-US", { numeric: "auto" });
  for (const [unit, size] of RELATIVE_STEPS) {
    if (Math.abs(delta) < size) return rtf.format(Math.round(delta), unit);
    delta /= size;
  }
  return DASH;
}

/** Timezone preference for absolute times: the browser's zone or UTC. */
export type TimeZonePref = "local" | "utc";

/** Absolute time, e.g. `Nov 10, 2026, 14:03:22 PST`, in the preferred zone. Components use
 *  `useDateTimeFormatter`, which supplies the stored preference. */
export function formatDateTime(date: Date | null, zone: TimeZonePref): string {
  if (!date) return DASH;
  return date.toLocaleString("en-US", {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
    timeZoneName: "short",
    timeZone: zone === "utc" ? "UTC" : undefined,
  });
}

/** Decimal gigabytes, the unit storage is priced in: `19,069 GB`, `0.99 GB`. */
export function formatGigabytes(bytes: number | null | undefined): string {
  if (bytes == null) return DASH;
  const gb = bytes / 1e9;
  return `${gb.toLocaleString("en-US", { maximumFractionDigits: gb >= 100 ? 0 : 2 })} GB`;
}

function currencyFormat(amount: number, currency: string, maxDigits: number): string {
  try {
    return amount.toLocaleString("en-US", {
      style: "currency",
      currency,
      minimumFractionDigits: 2,
      maximumFractionDigits: maxDigits,
    });
  } catch {
    // Not an ISO 4217 code: print the number with the code as given.
    return `${amount.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: maxDigits })} ${currency}`;
  }
}

/** Monthly cost in whole cents: `$255.29`. */
export function formatCost(amount: number | null | undefined, currency: string): string {
  if (amount == null) return DASH;
  return currencyFormat(amount, currency, 2);
}

/** Storage rate with its full precision: `$0.0133875 / GB-month`. */
export function formatRate(pricePerGbMonth: number | null | undefined, currency: string): string {
  if (pricePerGbMonth == null) return DASH;
  return `${currencyFormat(pricePerGbMonth, currency, 8)} / GB-month`;
}
