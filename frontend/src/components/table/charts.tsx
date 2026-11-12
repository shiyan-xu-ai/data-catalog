import { CartesianGrid, Line, LineChart, Pie, PieChart, XAxis, YAxis } from "recharts";
import { formatBytes, formatCompact, formatCount, formatPercent } from "@/lib/format";
import { useDateTimeFormatter } from "@/lib/preferences";
import type { VersionPoint } from "@/lib/table-metadata";
import { ChartContainer, ChartTooltip, ChartTooltipContent, type ChartConfig } from "@/components/ui/chart";

export interface StorageSlice {
  key: string;
  label: string;
  bytes: number;
  color: string;
}

/** Donut of byte slices with the total in the center; callers pass only non-zero slices. */
export function StorageSplitChart({
  slices,
  caption,
  className = "h-52",
}: {
  slices: StorageSlice[];
  caption: string;
  className?: string;
}) {
  const total = slices.reduce((s, x) => s + x.bytes, 0);
  const config = Object.fromEntries(slices.map((s) => [s.key, { label: s.label, color: s.color }])) satisfies ChartConfig;
  const data = slices.map((s) => ({ key: s.key, bytes: s.bytes, fill: `var(--color-${s.key})` }));
  return (
    <div className="relative">
      <ChartContainer config={config} className={`mx-auto aspect-square ${className}`}>
        <PieChart>
          <ChartTooltip
            content={
              <ChartTooltipContent
                hideLabel
                nameKey="key"
                valueFormatter={(v) => `${formatBytes(Number(v))} · ${formatPercent(total ? Number(v) / total : null)}`}
              />
            }
          />
          <Pie
            data={data}
            dataKey="bytes"
            nameKey="key"
            innerRadius="62%"
            outerRadius="95%"
            paddingAngle={slices.length > 1 ? 2 : 0}
            stroke="none"
            isAnimationActive={false}
          />
        </PieChart>
      </ChartContainer>
      <div className="pointer-events-none absolute inset-0 flex flex-col items-center justify-center">
        <span className="text-lg font-semibold tabular-nums">{formatBytes(total)}</span>
        <span className="text-xs text-muted-foreground">{caption}</span>
      </div>
    </div>
  );
}

const growthConfig = {
  rows: { label: "Rows", color: "hsl(var(--chart-1))" },
  bytes: { label: "Data size", color: "hsl(var(--chart-2))" },
} satisfies ChartConfig;

/** Rows and data size per version on independent axes. */
export function GrowthChart({ points }: { points: VersionPoint[] }) {
  const dots = points.length <= 60;
  const formatDateTime = useDateTimeFormatter();
  return (
    // Sticky above the version table, so it scales with the viewport and leaves room for rows.
    <ChartContainer config={growthConfig} className="aspect-auto h-[clamp(80px,15vh,160px)] w-full">
      <LineChart data={points} margin={{ left: 4, right: 4, top: 8, bottom: 0 }}>
        <CartesianGrid vertical={false} />
        <XAxis
          dataKey="version"
          tickLine={false}
          axisLine={false}
          fontSize={11}
          tickFormatter={(v: number) => `v${v}`}
          minTickGap={24}
        />
        <YAxis yAxisId="rows" tickLine={false} axisLine={false} fontSize={11} width={52} tickFormatter={formatCompact} />
        <YAxis
          yAxisId="bytes"
          orientation="right"
          tickLine={false}
          axisLine={false}
          fontSize={11}
          width={64}
          tickFormatter={formatBytes}
        />
        <ChartTooltip
          content={
            <ChartTooltipContent
              labelFormatter={(label, payload) => {
                const ts = payload[0]?.payload?.timestamp;
                return (
                  <span>
                    Version {String(label)}
                    {typeof ts === "number" && (
                      <span className="block font-normal text-muted-foreground">{formatDateTime(new Date(ts))}</span>
                    )}
                  </span>
                );
              }}
              valueFormatter={(v, key) => (key === "bytes" ? formatBytes(Number(v)) : formatCount(Number(v)))}
            />
          }
        />
        <Line
          yAxisId="rows"
          dataKey="rows"
          type="linear"
          stroke="var(--color-rows)"
          strokeWidth={2}
          dot={dots ? { r: 2.5, fill: "var(--color-rows)" } : false}
          isAnimationActive={false}
        />
        <Line
          yAxisId="bytes"
          dataKey="bytes"
          type="linear"
          stroke="var(--color-bytes)"
          strokeWidth={2}
          strokeDasharray="5 3"
          dot={dots ? { r: 2.5, fill: "var(--color-bytes)" } : false}
          isAnimationActive={false}
        />
      </LineChart>
    </ChartContainer>
  );
}

/** Inline legend swatch row used under charts. */
export function LegendItem({ color, label, value }: { color: string; label: string; value: string }) {
  return (
    <div className="flex items-center gap-2 text-xs">
      <span className="h-2.5 w-2.5 shrink-0 rounded-[2px]" style={{ background: color }} />
      <span className="min-w-0 flex-1 truncate text-muted-foreground">{label}</span>
      <span className="font-medium tabular-nums">{value}</span>
    </div>
  );
}
