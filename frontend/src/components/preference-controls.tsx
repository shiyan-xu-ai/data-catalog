import { useEffect, useState } from "react";
import { Monitor, Moon, Sun } from "lucide-react";
import type { TimeZonePref } from "@/lib/format";
import { setTimeZonePref, useTimeZonePref } from "@/lib/preferences";
import { applyTheme, getTheme, setTheme, type Theme } from "@/lib/theme";
import { cn } from "@/lib/utils";

function Segmented<T extends string>({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: T;
  options: { value: T; label: string; icon?: typeof Sun }[];
  onChange: (value: T) => void;
}) {
  return (
    <div className="flex items-center justify-between gap-3">
      <span className="text-xs text-muted-foreground">{label}</span>
      <div role="radiogroup" aria-label={label} className="inline-flex rounded-md border bg-background p-0.5">
        {options.map((o) => {
          const active = o.value === value;
          return (
            <button
              key={o.value}
              type="button"
              role="radio"
              aria-checked={active}
              aria-label={o.icon ? o.label : undefined}
              title={o.label}
              onClick={() => onChange(o.value)}
              className={cn(
                "flex h-6 items-center justify-center rounded-sm px-2 text-xs transition-colors",
                active ? "bg-accent font-medium text-accent-foreground shadow-xs" : "text-muted-foreground hover:text-foreground",
              )}
            >
              {o.icon ? <o.icon className="h-3.5 w-3.5" /> : o.label}
            </button>
          );
        })}
      </div>
    </div>
  );
}

const THEMES: { value: Theme; label: string; icon: typeof Sun }[] = [
  { value: "light", label: "Light", icon: Sun },
  { value: "dark", label: "Dark", icon: Moon },
  { value: "system", label: "System", icon: Monitor },
];

const ZONES: { value: TimeZonePref; label: string }[] = [
  { value: "utc", label: "UTC" },
  { value: "local", label: "Local" },
];

/** Theme and timezone controls, both persisted in localStorage. `compact` renders them as two
 *  icon buttons that cycle the choice, for the collapsed navigation rail. */
export function PreferenceControls({ compact = false }: { compact?: boolean }) {
  // The pre-paint script in index.html already applied the stored theme; sync state to it.
  const [theme, setThemeState] = useState<Theme>(getTheme);
  const timeZone = useTimeZonePref();

  // In system mode, follow live OS preference changes.
  useEffect(() => {
    if (theme !== "system") return;
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = () => applyTheme("system");
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, [theme]);

  function chooseTheme(t: Theme) {
    setTheme(t);
    setThemeState(t);
  }

  if (compact) {
    const current = THEMES.find((o) => o.value === theme) ?? THEMES[2];
    const nextTheme = THEMES[(THEMES.indexOf(current) + 1) % THEMES.length];
    const nextZone: TimeZonePref = timeZone === "utc" ? "local" : "utc";
    const railButton =
      "flex h-8 w-8 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-accent-foreground";
    return (
      <div className="flex flex-col items-center gap-1">
        <button
          type="button"
          className={railButton}
          onClick={() => chooseTheme(nextTheme.value)}
          aria-label={`Theme: ${current.label}. Switch to ${nextTheme.label}`}
          title={`Theme: ${current.label} (click for ${nextTheme.label})`}
        >
          <current.icon className="h-4 w-4" />
        </button>
        <button
          type="button"
          className={cn(railButton, "text-[10px] font-semibold")}
          onClick={() => setTimeZonePref(nextZone)}
          aria-label={`Timezone: ${timeZone === "utc" ? "UTC" : "local"}. Switch to ${nextZone === "utc" ? "UTC" : "local"}`}
          title={`Timezone: ${timeZone === "utc" ? "UTC" : "Local"} (click to switch)`}
        >
          {timeZone === "utc" ? "UTC" : "LOC"}
        </button>
      </div>
    );
  }

  return (
    <div className="space-y-2">
      <Segmented label="Theme" value={theme} options={THEMES} onChange={chooseTheme} />
      <Segmented label="Timezone" value={timeZone} options={ZONES} onChange={setTimeZonePref} />
    </div>
  );
}
