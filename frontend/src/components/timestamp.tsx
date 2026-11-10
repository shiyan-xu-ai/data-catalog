import { formatRelative, parseTime } from "@/lib/format";
import { useDateTimeFormatter } from "@/lib/preferences";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";

/** Relative time with the absolute time (in the preferred timezone) on hover; `stacked` also
 *  prints it below. */
export function Timestamp({ value, stacked = false }: { value: string | number | null | undefined; stacked?: boolean }) {
  const formatDateTime = useDateTimeFormatter();
  const date = parseTime(value);
  if (!date) return <span className="text-muted-foreground">—</span>;
  const absolute = formatDateTime(date);
  if (stacked) {
    return (
      <span className="flex flex-col">
        <span>{formatRelative(date)}</span>
        <span className="text-xs text-muted-foreground tabular-nums">{absolute}</span>
      </span>
    );
  }
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <time dateTime={date.toISOString()} className="cursor-default whitespace-nowrap">
          {formatRelative(date)}
        </time>
      </TooltipTrigger>
      <TooltipContent>{absolute}</TooltipContent>
    </Tooltip>
  );
}
