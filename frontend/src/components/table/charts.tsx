import { Bar, BarChart, CartesianGrid, Line, LineChart, Pie, PieChart, XAxis, YAxis } from "recharts";
import { formatBytes, formatCompact, formatCount, formatPercent } from "@/lib/format";
import { useDateTimeFormatter } from "@/lib/preferences";
import type { ColumnSlice, VersionPoint } from "@/lib/table-metadata";
import { ChartContainer, ChartTooltip, ChartTooltipContent, type ChartConfig } from "@/components/ui/chart";

const columnConfig = { bytes: { label: "Bytes on disk", color: "hsl(var(--chart-1))" } } satisfies ChartConfig;

/** Horizontal bars of bytes on disk per top-level column, largest first. */
export function ColumnStorageChart({ columns }: { columns: ColumnSlice[] }) {
  const total = columns.reduce((s, c) => s + c.bytes, 0);
  const data = columns.map((c) => ({
    ...c,
    fill: c.other ? "hsl(var(--chart-6))" : "var(--color-bytes)",
  }));
  return (
    <ChartContainer config={columnConfig} className="aspect-auto w-full" style={{ height: data.length * 30 + 24 }}>
      <BarChart data={data} layout="vertical" margin={{ left: 4, right: 56, top: 0, bottom: 0 }}>
        <CartesianGrid horizontal={false} />
        <XAxis type="number" dataKey="bytes" tickFormatter={formatBytes} tickLine={false} axisLine={false} fontSize={11} />
        <YAxis
          type="category"
          dataKey="name"
          width={150}
          tickLine={false}
          axisLine={false}
          interval={0}
          fontSize={11}
          tickFormatter={(v: string) => (v.length > 22 ? `${v.slice(0, 21)}…` : v)}
        />
        <ChartTooltip
          cursor={false}
          content={
            <ChartTooltipContent
              valueFormatter={(v) => `${formatBytes(Number(v))} · ${formatPercent(total ? Number(v) / total : null)}`}
            />
          }
        />
        <Bar
          dataKey="bytes"
          radius={3}
          barSize={18}
          isAnimationActive={false}
          label={{
            position: "right",
            fontSize: 11,
            className: "fill-muted-foreground",
            formatter: (v: unknown) => formatBytes(Number(v)),
          }}
        />
      </BarChart>
    </ChartContainer>
  );
}

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
    <ChartContainer config={growthConfig} className="aspect-auto h-64 w-full">
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
export function LegendItem({ color, label, value, note }: { color: string; label: string; value: string; note?: string }) {
  return (
    <div className="flex items-start gap-2 text-xs">
      <span className="mt-0.5 h-2.5 w-2.5 shrink-0 rounded-[2px]" style={{ background: color }} />
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline justify-between gap-3">
          <span className="text-muted-foreground">{label}</span>
          <span className="font-medium tabular-nums">{value}</span>
        </div>
        {note && <div className="text-[11px] text-muted-foreground/80">{note}</div>}
      </div>
    </div>
  );
}
