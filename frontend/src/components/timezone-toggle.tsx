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
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button variant="ghost" size="icon" onClick={toggle} aria-label={`Timezone: ${label}`}>
          <Globe className="h-4 w-4" />
        </Button>
      </TooltipTrigger>
      <TooltipContent>Timezone: {label}</TooltipContent>
    </Tooltip>
  );
}
