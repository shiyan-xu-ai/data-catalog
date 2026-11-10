import { formatCompact, formatCount, formatPercent } from "@/lib/format";
import { COMPACTION_TARGET_ROWS, fragmentBoxPlot, type FragmentLengths } from "@/lib/table-metadata";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";

const ACCENT = "hsl(var(--chart-1))";
const ACCENT_FILL = "hsl(var(--chart-1) / 0.25)";
const WHISKER = "hsl(var(--muted-foreground))";
const INK = "hsl(var(--foreground))";

const PLOT_HEIGHT = 36;
const MID = PLOT_HEIGHT / 2;
const BOX_HALF = 10;

type Kind = "cap" | "box" | "median" | "p99" | "mean";

/** Glyph for each mark, drawn around (0, 0) so the plot and the legend swatches share it. */
function Glyph({ kind }: { kind: Kind }) {
  switch (kind) {
    case "cap":
      return <line y1={-6} y2={6} stroke={WHISKER} strokeWidth={1} />;
    case "box":
      return <rect x={-5} y={-5} width={10} height={10} rx={1} fill={ACCENT_FILL} stroke={ACCENT} strokeWidth={1} />;
    case "median":
      return <line y1={-BOX_HALF} y2={BOX_HALF} stroke={ACCENT} strokeWidth={2} />;
    case "p99":
      return <line y1={-5} y2={5} stroke={INK} strokeWidth={1} />;
    case "mean":
      return <polygon points="0,-4.5 4.5,0 0,4.5 -4.5,0" fill="hsl(var(--card))" stroke={INK} strokeWidth={1.25} />;
  }
}

function Swatch({ kind }: { kind: Kind }) {
  return (
    <svg viewBox="-6 -6 12 12" className="h-3 w-3 shrink-0" aria-hidden>
      {kind === "median" ? <line y1={-6} y2={6} stroke={ACCENT} strokeWidth={2} /> : <Glyph kind={kind} />}
    </svg>
  );
}

/** Hover/focus target for one plot mark; the tooltip names the mark and its value. */
function Mark({ label, value, children }: { label: string; value: string; children: React.ReactNode }) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <g tabIndex={0} aria-label={`${label}: ${value}`} className="cursor-default outline-none">
          {children}
        </g>
      </TooltipTrigger>
      <TooltipContent side="top" className="tabular-nums">
        <span className="font-medium">{label}</span> {value}
      </TooltipContent>
    </Tooltip>
  );
}

/** Rows-per-fragment distribution: median headline, a range plot (whisker min to max, box p25
 *  to p75, median line, mean diamond, p99 tick), and a definition grid of the exact values. */
export function FragmentBoxPlot({ lengths }: { lengths: FragmentLengths }) {
  const { ticks, axisMax, pct } = fragmentBoxPlot(lengths);
  const rows = (v: number) => `${formatCount(v)} rows`;
  // Drawn in this order so the smaller marks sit on top of (and win hover over) the box.
  const points: { key: string; label: string; kind: Kind; at: number; value: number }[] = [
    { key: "min", label: "Min", kind: "cap", at: pct.min, value: lengths.min },
    { key: "max", label: "Max", kind: "cap", at: pct.max, value: lengths.max },
    { key: "p99", label: "P99", kind: "p99", at: pct.p99, value: lengths.p99 },
    { key: "median", label: "Median", kind: "median", at: pct.p50, value: lengths.p50 },
    { key: "mean", label: "Mean", kind: "mean", at: pct.mean, value: lengths.mean },
  ];
  const cells: { label: string; kind: Kind; value: number }[] = [
    { label: "Min", kind: "cap", value: lengths.min },
    { label: "P25", kind: "box", value: lengths.p25 },
    { label: "Median", kind: "median", value: lengths.p50 },
    { label: "P75", kind: "box", value: lengths.p75 },
    { label: "P99", kind: "p99", value: lengths.p99 },
    { label: "Max", kind: "cap", value: lengths.max },
    { label: "Mean", kind: "mean", value: lengths.mean },
  ];

  return (
    <figure className="@container min-w-0 space-y-4" aria-label="Rows per fragment">
      <div>
        <p className="flex flex-wrap items-baseline gap-x-1.5">
          <span className="text-sm text-muted-foreground">Median</span>
          <span className="text-2xl font-semibold tracking-tight tabular-nums">{formatCount(lengths.p50)}</span>
          <span className="text-sm text-muted-foreground">rows per fragment</span>
        </p>
        <p className="text-xs text-muted-foreground">physical rows, current version</p>
      </div>

      <div className="px-1.5">
        <svg width="100%" height={PLOT_HEIGHT} overflow="visible" role="img" aria-label={`Rows per fragment, axis 0 to ${formatCount(axisMax)}`}>
          {ticks.map((t) => (
            <line
              key={t}
              x1={`${(t / axisMax) * 100}%`}
              x2={`${(t / axisMax) * 100}%`}
              y1={0}
              y2={PLOT_HEIGHT}
              stroke="hsl(var(--border))"
              strokeWidth={1}
            />
          ))}
          <line x1={`${pct.min}%`} x2={`${pct.max}%`} y1={MID} y2={MID} stroke={WHISKER} strokeWidth={1} pointerEvents="none" />
          <Mark label="P25–P75" value={`${formatCount(lengths.p25)} – ${formatCount(lengths.p75)} rows`}>
            <rect
              x={`${pct.p25}%`}
              width={`${pct.p75 - pct.p25}%`}
              y={MID - BOX_HALF}
              height={BOX_HALF * 2}
              rx={2}
              fill={ACCENT_FILL}
              stroke={ACCENT}
              strokeWidth={1}
            />
          </Mark>
          {points.map((p) => (
            <Mark key={p.key} label={p.label} value={rows(p.value)}>
              <svg x={`${p.at}%`} y={MID} overflow="visible">
                <rect x={-5} y={-MID} width={10} height={PLOT_HEIGHT} fill="transparent" />
                <Glyph kind={p.kind} />
              </svg>
            </Mark>
          ))}
        </svg>
        <div className="relative mt-1.5 h-4 text-[11px] text-muted-foreground tabular-nums" aria-hidden>
          {ticks.map((t, i) => (
            <span
              key={t}
              className={
                i === 0 ? "absolute" : i === ticks.length - 1 ? "absolute -translate-x-full" : "absolute -translate-x-1/2"
              }
              style={{ left: `${(t / axisMax) * 100}%` }}
            >
              {formatCompact(t)}
            </span>
          ))}
        </div>
      </div>

      <dl className="grid grid-cols-4 gap-1.5 @md:grid-cols-7">
        {cells.map((c) => (
          <div key={c.label} className="min-w-0 rounded-md bg-muted/50 px-2 py-1.5">
            <dt className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
              <Swatch kind={c.kind} />
              {c.label}
            </dt>
            <dd className="truncate text-sm font-medium tabular-nums">{formatCount(c.value)}</dd>
          </div>
        ))}
      </dl>
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
          <span className="tabular-nums">{formatCount(total)}</span> fragments have fewer than{" "}
          <span className="tabular-nums">{formatCount(COMPACTION_TARGET_ROWS)}</span> rows
        </p>
        <p className="text-xs text-muted-foreground">
          Lance&apos;s default compaction target (CompactionOptions.target_rows_per_fragment); fragments below it are
          compaction candidates.
        </p>
        {targetOverride != null && (
          <p className="text-xs text-muted-foreground">
            Table override: <span className="tabular-nums">{formatCount(targetOverride)}</span> rows. GetTableStats counts
            against the fixed default.
          </p>
        )}
      </div>
    </div>
  );
}
