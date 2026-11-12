import { formatBytes, formatCount, formatPercent } from "@/lib/format";
import type { Distribution } from "@/lib/schemas";
import { Skeleton } from "@/components/ui/skeleton";
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

/** One per-fragment measure: its distribution, or loading / unavailable / not recorded. */
export interface FragmentMeasure {
  label: string;
  value: Distribution | null | undefined;
  bytes?: boolean;
  loading: boolean;
  error?: string;
}

/** Per-fragment statistics, one row per measure and one column per statistic. */
export function FragmentStatsTable({ measures }: { measures: FragmentMeasure[] }) {
  return (
    <Table aria-label="Per-fragment statistics">
      <TableHeader>
        <TableRow className="hover:bg-transparent">
          <TableHead>Per fragment</TableHead>
          {STATS.map(([label]) => (
            <TableHead key={label} className="text-right">
              {label}
            </TableHead>
          ))}
        </TableRow>
      </TableHeader>
      <TableBody>
        {measures.map((m) => (
          <TableRow key={m.label} className="hover:bg-transparent">
            <TableCell className="whitespace-nowrap">{m.label}</TableCell>
            <MeasureCells measure={m} />
          </TableRow>
        ))}
      </TableBody>
    </Table>
  );
}

function MeasureCells({ measure: { value, bytes, loading, error } }: { measure: FragmentMeasure }) {
  if (loading) {
    return (
      <TableCell colSpan={STATS.length}>
        <Skeleton className="h-4 w-full" />
      </TableCell>
    );
  }
  if (error || !value) {
    return (
      <TableCell colSpan={STATS.length} className="text-right text-xs text-muted-foreground" title={error}>
        {error ? "Unavailable" : "Not recorded"}
      </TableCell>
    );
  }
  return STATS.map(([label, key]) => (
    <TableCell key={label} className="text-right tabular-nums">
      {bytes ? formatBytes(value[key]) : formatCount(value[key])}
    </TableCell>
  ));
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
    <div className="flex items-center gap-4">
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
        <p className="text-sm font-medium">Small fragments</p>
        {targetOverride != null && (
          <p className="text-xs text-muted-foreground">
            Table target: <span className="tabular-nums">{formatCount(targetOverride)}</span> rows.
          </p>
        )}
      </div>
    </div>
  );
}
