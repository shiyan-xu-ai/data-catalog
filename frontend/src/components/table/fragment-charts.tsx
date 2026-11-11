import { Bar, BarChart, CartesianGrid, Cell, XAxis, YAxis } from "recharts";
import { formatCompact, formatCount, formatPercent } from "@/lib/format";
import { COMPACTION_TARGET_ROWS, niceAxis, type FragmentLengths } from "@/lib/table-metadata";
import { ChartContainer, ChartTooltip, ChartTooltipContent, type ChartConfig } from "@/components/ui/chart";

// Same chart config/styling as ColumnStorageChart so the Overview bars read alike; the mean
// gets its own color because it is not a percentile.
const rowsConfig = {
  rows: { label: "Rows per fragment", color: "hsl(var(--chart-1))" },
  mean: { label: "Mean", color: "hsl(var(--chart-2))" },
} satisfies ChartConfig;

const STATS = [
  ["Min", "min"],
  ["P25", "p25"],
  ["Median", "p50"],
  ["Mean", "mean"],
  ["P75", "p75"],
  ["P99", "p99"],
  ["Max", "max"],
] as const;

/** Rows-per-fragment distribution: one horizontal bar per statistic, ordered by value
 *  (ties keep percentile order), with the mean highlighted. */
export function FragmentDistributionChart({ lengths }: { lengths: FragmentLengths }) {
  const { axisMax, ticks } = niceAxis(lengths.max);
  const data = STATS.map(([name, key]) => ({ name, rows: lengths[key], isMean: key === "mean" })).sort(
    (a, b) => a.rows - b.rows,
  );
  return (
    <figure className="min-w-0" aria-label="Rows per fragment">
      <ChartContainer config={rowsConfig} className="aspect-auto w-full" style={{ height: data.length * 30 + 24 }}>
        <BarChart data={data} layout="vertical" margin={{ left: 4, right: 56, top: 0, bottom: 0 }}>
          <CartesianGrid horizontal={false} />
          <XAxis
            type="number"
            dataKey="rows"
            domain={[0, axisMax]}
            ticks={ticks}
            tickFormatter={formatCompact}
            tickLine={false}
            axisLine={false}
            fontSize={11}
          />
          <YAxis type="category" dataKey="name" width={56} tickLine={false} axisLine={false} interval={0} fontSize={11} />
          <ChartTooltip cursor={false} content={<ChartTooltipContent valueFormatter={(v) => `${formatCount(Number(v))} rows`} />} />
          <Bar
            dataKey="rows"
            fill="var(--color-rows)"
            radius={3}
            barSize={18}
            isAnimationActive={false}
            label={{
              position: "right",
              fontSize: 11,
              className: "fill-muted-foreground tabular-nums",
              formatter: (v: unknown) => formatCount(Number(v)),
            }}
          >
            {data.map((d) => (
              <Cell key={d.name} fill={d.isMean ? "var(--color-mean)" : "var(--color-rows)"} />
            ))}
          </Bar>
        </BarChart>
      </ChartContainer>
    </figure>
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
        <p className="text-xs text-muted-foreground">
          Lance&apos;s default compaction target. Fragments below it are compaction candidates.
        </p>
        {targetOverride != null && (
          <p className="text-xs text-muted-foreground">
            This table overrides the target to <span className="tabular-nums">{formatCount(targetOverride)}</span> rows.
            The count above uses the default.
          </p>
        )}
      </div>
    </div>
  );
}
