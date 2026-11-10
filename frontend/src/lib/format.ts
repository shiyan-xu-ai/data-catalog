export function formatCount(n: number | null | undefined): string {
  if (n == null) return "—";
  return n.toLocaleString();
}
