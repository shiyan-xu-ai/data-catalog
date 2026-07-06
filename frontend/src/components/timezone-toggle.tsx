import { Globe } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { useTz } from "@/lib/timezone";

const LABEL = { utc: "UTC", local: "Local" };

export function TimezoneToggle() {
  const { mode, toggle } = useTz();
  const label = LABEL[mode];
  const other = mode === "utc" ? LABEL.local : LABEL.utc;
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          variant="ghost"
          size="sm"
          onClick={toggle}
          aria-label={`Timezone: ${label}, click to switch to ${other}`}
        >
          <Globe className="h-4 w-4" />
          {label}
        </Button>
      </TooltipTrigger>
      <TooltipContent>Times shown in {label} — click to switch to {other}</TooltipContent>
    </Tooltip>
  );
}
