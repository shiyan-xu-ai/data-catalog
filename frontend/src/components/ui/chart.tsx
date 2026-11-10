import * as React from "react";
import * as RechartsPrimitive from "recharts";
import { cn } from "@/lib/utils";

// shadcn/ui chart pattern: a config maps series keys to labels and theme-aware colors, exposed
// to Recharts as `var(--color-<key>)` on the chart container.

export type ChartConfig = Record<string, { label?: React.ReactNode; color?: string }>;

const ChartContext = React.createContext<{ config: ChartConfig } | null>(null);

function useChart() {
  const ctx = React.useContext(ChartContext);
  if (!ctx) throw new Error("useChart must be used within a <ChartContainer />");
  return ctx;
}

export function ChartContainer({
  id,
  config,
  className,
  children,
  ...props
}: React.ComponentProps<"div"> & {
  config: ChartConfig;
  children: React.ComponentProps<typeof RechartsPrimitive.ResponsiveContainer>["children"];
}) {
  const uniqueId = React.useId();
  const chartId = `chart-${id ?? uniqueId.replace(/:/g, "")}`;
  const vars = Object.entries(config)
    .filter(([, c]) => c.color)
    .map(([key, c]) => `--color-${key}: ${c.color};`)
    .join(" ");
  return (
    <ChartContext.Provider value={{ config }}>
      <div
        data-chart={chartId}
        className={cn(
          "flex aspect-video justify-center text-xs [&_.recharts-cartesian-axis-tick_text]:fill-muted-foreground [&_.recharts-cartesian-grid_line[stroke='#ccc']]:stroke-border/60 [&_.recharts-curve.recharts-tooltip-cursor]:stroke-border [&_.recharts-rectangle.recharts-tooltip-cursor]:fill-muted/60 [&_.recharts-sector]:outline-hidden [&_.recharts-surface]:outline-hidden [&_.recharts-layer]:outline-hidden",
          className,
        )}
        {...props}
      >
        <style>{`[data-chart=${chartId}] { ${vars} }`}</style>
        <RechartsPrimitive.ResponsiveContainer>{children}</RechartsPrimitive.ResponsiveContainer>
      </div>
    </ChartContext.Provider>
  );
}

export const ChartTooltip = RechartsPrimitive.Tooltip;

interface TooltipEntry {
  dataKey?: string | number | ((obj: unknown) => unknown);
  name?: string | number;
  value?: unknown;
  color?: string;
  payload?: Record<string, unknown> & { fill?: string };
}

/** Tooltip body. Recharts injects `active`, `payload`, and `label` when used as `content`. */
export function ChartTooltipContent({
  active,
  payload,
  label,
  hideLabel = false,
  nameKey,
  labelFormatter,
  valueFormatter,
}: {
  active?: boolean;
  payload?: readonly TooltipEntry[];
  label?: unknown;
  hideLabel?: boolean;
  /** Payload field naming each entry (pie slices), instead of the series key. */
  nameKey?: string;
  labelFormatter?: (label: unknown, payload: readonly TooltipEntry[]) => React.ReactNode;
  valueFormatter?: (value: unknown, key: string) => React.ReactNode;
}) {
  const { config } = useChart();
  if (!active || !payload?.length) return null;
  return (
    <div className="grid min-w-36 gap-1.5 rounded-lg border bg-popover px-2.5 py-1.5 text-xs text-popover-foreground shadow-lg">
      {!hideLabel && (
        <div className="font-medium">{labelFormatter ? labelFormatter(label, payload) : String(label ?? "")}</div>
      )}
      {payload.map((item, i) => {
        const key = String(nameKey ? item.payload?.[nameKey] : (item.dataKey ?? item.name ?? i));
        const color = item.payload?.fill ?? item.color;
        return (
          <div key={key} className="flex items-center gap-2">
            <span className="h-2.5 w-2.5 shrink-0 rounded-[2px]" style={{ background: color }} />
            <span className="text-muted-foreground">{config[key]?.label ?? key}</span>
            <span className="ml-auto pl-3 font-mono font-medium tabular-nums">
              {valueFormatter ? valueFormatter(item.value, key) : String(item.value)}
            </span>
          </div>
        );
      })}
    </div>
  );
}
