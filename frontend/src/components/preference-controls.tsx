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

/** Theme and timezone controls, both persisted in localStorage. */
export function PreferenceControls() {
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

  return (
    <div className="space-y-2">
      <Segmented
        label="Theme"
        value={theme}
        options={THEMES}
        onChange={(t) => {
          setTheme(t);
          setThemeState(t);
        }}
      />
      <Segmented label="Timezone" value={timeZone} options={ZONES} onChange={setTimeZonePref} />
    </div>
  );
}
