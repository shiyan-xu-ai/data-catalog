import { formatCompact, formatCount, formatPercent } from "@/lib/format";
import { COMPACTION_TARGET_ROWS, fragmentBoxPlot, type FragmentLengths } from "@/lib/table-metadata";

const BOX = "hsl(var(--chart-1))";
const P99 = "hsl(var(--chart-2))";
const MEAN = "hsl(var(--chart-3))";
const WHISKER = "hsl(var(--muted-foreground) / 0.6)";

/** Horizontal box plot of physical rows per fragment: whiskers span min to max, the box spans
 *  p25 to p75 with the median line, plus p99 and mean markers. */
export function FragmentBoxPlot({ lengths }: { lengths: FragmentLengths }) {
  const { ticks, pct } = fragmentBoxPlot(lengths);
  const stats: { label: string; value: number; color: string; shape: "line" | "box" | "diamond" | "tick" }[] = [
    { label: "Min", value: lengths.min, color: WHISKER, shape: "line" },
    { label: "P25", value: lengths.p25, color: BOX, shape: "box" },
    { label: "Median", value: lengths.p50, color: BOX, shape: "line" },
    { label: "Mean", value: lengths.mean, color: MEAN, shape: "diamond" },
    { label: "P75", value: lengths.p75, color: BOX, shape: "box" },
    { label: "P99", value: lengths.p99, color: P99, shape: "tick" },
    { label: "Max", value: lengths.max, color: WHISKER, shape: "line" },
  ];
  return (
    <figure className="space-y-3" aria-label="Rows per fragment">
      <div className="px-2">
        <div className="relative h-14">
          <div className="absolute top-1/2 h-px" style={{ left: `${pct.min}%`, width: `${pct.max - pct.min}%`, background: WHISKER }} />
          {[pct.min, pct.max].map((p, i) => (
            <div key={i} className="absolute top-1/2 h-5 w-px -translate-y-1/2" style={{ left: `${p}%`, background: WHISKER }} />
          ))}
          <div
            className="absolute top-2 bottom-2 rounded-sm border"
            style={{
              left: `${pct.p25}%`,
              width: `max(3px, ${pct.p75 - pct.p25}%)`,
              borderColor: BOX,
              background: "hsl(var(--chart-1) / 0.15)",
            }}
          />
          <div className="absolute top-2 bottom-2 w-0.5 -translate-x-1/2" style={{ left: `${pct.p50}%`, background: BOX }} />
          <div className="absolute top-1 h-3 w-0.5 -translate-x-1/2" style={{ left: `${pct.p99}%`, background: P99 }} />
          <div
            className="absolute bottom-0.5 h-2.5 w-2.5 -translate-x-1/2 rotate-45"
            style={{ left: `${pct.mean}%`, background: MEAN }}
          />
        </div>
        <div className="relative mt-1 h-4 border-t text-[10px] text-muted-foreground tabular-nums">
          {ticks.map((t, i) => (
            <span
              key={t}
              className={i === 0 ? "absolute top-1" : i === ticks.length - 1 ? "absolute top-1 -translate-x-full" : "absolute top-1 -translate-x-1/2"}
              style={{ left: `${(i / (ticks.length - 1)) * 100}%` }}
            >
              {formatCompact(t)}
            </span>
          ))}
        </div>
      </div>
      <figcaption className="grid grid-cols-4 gap-x-4 gap-y-1.5 text-xs @2xl:grid-cols-7">
        {stats.map((s) => (
          <div key={s.label} className="flex items-center gap-1.5">
            <span
              className={
                s.shape === "diamond"
                  ? "h-2 w-2 shrink-0 rotate-45"
                  : s.shape === "box"
                    ? "h-2.5 w-2.5 shrink-0 rounded-[2px] border"
                    : s.shape === "tick"
                      ? "h-3 w-0.5 shrink-0"
                      : "h-0.5 w-3 shrink-0"
              }
              style={
                s.shape === "box"
                  ? { borderColor: s.color, background: "hsl(var(--chart-1) / 0.15)" }
                  : { background: s.color }
              }
            />
            <span className="text-muted-foreground">{s.label}</span>
            <span className="ml-auto font-medium tabular-nums">{formatCount(s.value)}</span>
          </div>
        ))}
      </figcaption>
    </figure>
  );
}

/** Share of fragments below Lance's compaction target, as a ring. */
export function SmallFragmentShare({ small, total }: { small: number; total: number }) {
  const fraction = total > 0 ? small / total : 0;
  const r = 34;
  const circumference = 2 * Math.PI * r;
  return (
    <div className="flex items-center gap-4">
      <svg viewBox="0 0 88 88" className="h-24 w-24 shrink-0 -rotate-90" role="img" aria-label={`${formatPercent(fraction)} small fragments`}>
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
      <div className="min-w-0 space-y-1 text-sm">
        <p>
          <span className="font-semibold tabular-nums">{formatCount(small)}</span> of{" "}
          <span className="tabular-nums">{formatCount(total)}</span> fragments are small
        </p>
        <p className="text-xs text-muted-foreground">Fewer than {formatCount(COMPACTION_TARGET_ROWS)} rows each</p>
        {total > 1 && small === total && (
          <p className="text-xs text-muted-foreground">
            Every fragment is below the compaction target, so compacting would merge them into fewer fragments.
          </p>
        )}
      </div>
    </div>
  );
}
