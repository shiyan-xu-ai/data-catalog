// Small formatting helpers.

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB", "PB"];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(1)} ${units[i]}`;
}

export function formatTime(s: string | null): string {
  if (s === null) return "—";
  const d = new Date(s);
  if (Number.isNaN(d.getTime())) return s;
  return d.toISOString().replace("T", " ").replace(/\.\d+Z$/, "Z");
}

export function nsToString(ns: string[]): string {
  return ns.length === 0 ? "—" : ns.join("/");
}

export function ttlPolicyString(p: { keep_last_n: number | null; max_age_days: number | null } | null): string {
  if (p === null) return "—";
  const parts: string[] = [];
  if (p.keep_last_n !== null) parts.push(`keep_last_n=${p.keep_last_n}`);
  if (p.max_age_days !== null) parts.push(`max_age_days=${p.max_age_days}`);
  return parts.length === 0 ? "no policy" : parts.join(", ");
}
