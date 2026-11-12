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

interface ZonedParts {
  date: string;
  time: string;
  millis: string;
  offset: string;
}

const pad = (n: number, width = 2) => String(n).padStart(width, "0");

/** Wall-clock parts of `date` in the preferred zone. `localZone` overrides the browser's zone
 *  for `local` (tests pin it). */
function zonedParts(date: Date, zone: TimeZonePref, localZone?: string): ZonedParts {
  const timeZone = zone === "utc" ? "UTC" : localZone;
  const parts = new Intl.DateTimeFormat("en-US", {
    timeZone,
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hourCycle: "h23",
  }).formatToParts(date);
  const get = (type: Intl.DateTimeFormatPartTypes) => Number(parts.find((p) => p.type === type)?.value);
  const [y, mo, d, h, mi, s] = [get("year"), get("month"), get("day"), get("hour"), get("minute"), get("second")];
  const ms = date.getUTCMilliseconds();
  // The zone's offset is how far its wall clock runs ahead of UTC at this instant.
  const offsetMin = Math.round((Date.UTC(y, mo - 1, d, h, mi, s, ms) - date.getTime()) / 60_000);
  const abs = Math.abs(offsetMin);
  return {
    date: `${pad(y, 4)}-${pad(mo)}-${pad(d)}`,
    time: `${pad(h)}:${pad(mi)}:${pad(s)}`,
    millis: pad(ms, 3),
    offset: offsetMin === 0 ? "Z" : `${offsetMin < 0 ? "-" : "+"}${pad(Math.floor(abs / 60))}:${pad(abs % 60)}`,
  };
}

/** `2026-11-10` in the preferred zone. */
export function formatIsoDate(date: Date | null, zone: TimeZonePref, localZone?: string): string {
  return date ? zonedParts(date, zone, localZone).date : DASH;
}

/** `21:02:37` (24-hour) in the preferred zone. */
export function formatIsoTime(date: Date | null, zone: TimeZonePref, localZone?: string): string {
  return date ? zonedParts(date, zone, localZone).time : DASH;
}

/** Full ISO 8601 with milliseconds and offset in the preferred zone:
 *  `2026-11-10T21:02:37.123Z`, `2026-11-10T13:02:37.123-08:00`. */
export function formatIsoTimestamp(date: Date | null, zone: TimeZonePref, localZone?: string): string {
  if (!date) return DASH;
  const p = zonedParts(date, zone, localZone);
  return `${p.date}T${p.time}.${p.millis}${p.offset}`;
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
