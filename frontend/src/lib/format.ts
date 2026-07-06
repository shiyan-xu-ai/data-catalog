import type { TtlPolicy } from "./schemas";
import { getTz, type TzMode } from "./timezone";

export function formatBytes(n: number | null | undefined): string {
  if (n == null || n === 0) return "—";
  const units = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
  let v = n;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  const str = i === 0 || v >= 100 ? String(Math.round(v)) : v.toFixed(1).replace(/\.0$/, "");
  return `${str} ${units[i]}`;
}

export function formatCount(n: number | null | undefined): string {
  if (n == null) return "—";
  return n.toLocaleString();
}

export function formatTime(iso: string | null | undefined, mode: TzMode = getTz()): string {
  if (!iso) return "—";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return new Intl.DateTimeFormat(undefined, {
    year: "numeric",
    month: "short",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    timeZoneName: "shortOffset",
    ...(mode === "utc" ? { timeZone: "UTC" } : {}),
  }).format(d);
}

export function formatRelative(iso: string | null | undefined): string {
  if (!iso) return "—";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  const diff = d.getTime() - Date.now();
  const abs = Math.abs(diff);
  const past = diff < 0;
  const mins = Math.round(abs / 60000);
  if (mins < 1) return past ? "just now" : "in a moment";
  if (mins < 60) return past ? `${mins}m ago` : `in ${mins}m`;
  const hrs = Math.round(mins / 60);
  if (hrs < 48) return past ? `${hrs}h ago` : `in ${hrs}h`;
  const days = Math.round(hrs / 24);
  if (days < 30) return past ? `${days}d ago` : `in ${days}d`;
  return formatTime(iso);
}

export function nsToString(segments: string[]): string {
  return segments.join(".");
}

export function ttlPolicyString(p: TtlPolicy | null | undefined): string {
  if (!p) return "none";
  const parts: string[] = [];
  if (p.keep_last_n != null) parts.push(`keep last ${p.keep_last_n}`);
  if (p.max_age_days != null) parts.push(`max age ${p.max_age_days}d`);
  return parts.length ? parts.join(", ") : "none";
}

export function shapeLabel(s: string): string {
  switch (s) {
    case "full": return "full";
    case "lance_only": return "lance only";
    case "seg_only": return "segments only";
    case "lance_only_partial": return "lance partial";
    case "empty": return "empty";
    default: return s;
  }
}
