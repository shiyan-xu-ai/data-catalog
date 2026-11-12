import { formatCount, formatPercent } from "@/lib/format";
import { COMPACTION_TARGET_ROWS, type FragmentLengths } from "@/lib/table-metadata";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

const STATS = [
  ["Min", "min"],
  ["P25", "p25"],
  ["Median", "p50"],
  ["Mean", "mean"],
  ["P75", "p75"],
  ["P99", "p99"],
  ["Max", "max"],
] as const;

/** Rows-per-fragment statistics, one column each. */
export function FragmentStatsTable({ lengths }: { lengths: FragmentLengths }) {
  return (
    <Table aria-label="Rows per fragment">
      <TableHeader>
        <TableRow className="hover:bg-transparent">
          {STATS.map(([label]) => (
            <TableHead key={label} className="text-right">
              {label}
            </TableHead>
          ))}
        </TableRow>
      </TableHeader>
      <TableBody>
        <TableRow className="hover:bg-transparent">
          {STATS.map(([label, key]) => (
            <TableCell key={label} className="text-right tabular-nums">
              {formatCount(lengths[key])}
            </TableCell>
          ))}
        </TableRow>
      </TableBody>
    </Table>
  );
}

/** Share of fragments below Lance's default compaction target, as a ring. */
export function SmallFragmentShare({
  small,
  total,
  targetOverride,
}: {
  small: number;
  total: number;
  /** The table's `lance.compaction.target_rows_per_fragment`, if set. */
  targetOverride: number | null;
}) {
  const fraction = total > 0 ? small / total : 0;
  const r = 34;
  const circumference = 2 * Math.PI * r;
  return (
    <div className="flex items-start gap-4">
      <svg viewBox="0 0 88 88" className="h-20 w-20 shrink-0 -rotate-90" role="img" aria-label={`${formatPercent(fraction)} small fragments`}>
        <circle cx="44" cy="44" r={r} fill="none" stroke="hsl(var(--muted))" strokeWidth="9" />
        <circle
          cx="44"
          cy="44"
          r={r}
          fill="none"
          stroke="hsl(var(--chart-3))"
          strokeWidth="9"
          strokeLinecap={fraction > 0 && fraction < 1 ? "round" : "butt"}
          strokeDasharray={`${fraction * circumference} ${circumference}`}
        />
        <text
          x="44"
          y="44"
          className="rotate-90 fill-foreground text-[15px] font-semibold tabular-nums"
          style={{ transformOrigin: "44px 44px" }}
          textAnchor="middle"
          dominantBaseline="central"
        >
          {formatPercent(fraction)}
        </text>
      </svg>
      <div className="min-w-0 space-y-1.5">
        <p className="text-sm">
          <span className="font-semibold tabular-nums">{formatCount(small)}</span> of{" "}
          <span className="tabular-nums">{formatCount(total)}</span> {total === 1 ? "fragment has" : "fragments have"}{" "}
          fewer than{" "}
          <span className="tabular-nums">{formatCount(COMPACTION_TARGET_ROWS)}</span> rows
        </p>
        {targetOverride != null && (
          <p className="text-xs text-muted-foreground">
            Table target: <span className="tabular-nums">{formatCount(targetOverride)}</span> rows.
          </p>
        )}
      </div>
    </div>
  );
}
