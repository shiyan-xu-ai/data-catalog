import { useEffect, useState } from "react";
import { Sun, Moon, Monitor } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { getTheme, setTheme, applyTheme, type Theme } from "@/lib/theme";

const ORDER: Theme[] = ["light", "dark", "system"];
const ICON = { light: Sun, dark: Moon, system: Monitor };
const LABEL = { light: "Light", dark: "Dark", system: "System" };

export function ThemeToggle() {
  const [theme, setThemeState] = useState<Theme>("system");

  // Sync from storage after mount (the pre-paint script already applied the class).
  useEffect(() => setThemeState(getTheme()), []);

  // In system mode, track live OS preference changes.
  useEffect(() => {
    if (theme !== "system") return;
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = () => applyTheme("system");
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, [theme]);

  const cycle = () => {
    const next = ORDER[(ORDER.indexOf(theme) + 1) % ORDER.length];
    setTheme(next);
    setThemeState(next);
  };

  const Icon = ICON[theme];
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button variant="ghost" size="icon" onClick={cycle} aria-label={`Theme: ${LABEL[theme]}`}>
          <Icon className="h-4 w-4" />
        </Button>
      </TooltipTrigger>
      <TooltipContent>Theme: {LABEL[theme]}</TooltipContent>
    </Tooltip>
  );
}
