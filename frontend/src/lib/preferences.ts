import { useEffect, useState, useSyncExternalStore, type Dispatch, type SetStateAction } from "react";
import { formatDateTime, type TimeZonePref } from "./format";

// Display preferences persisted in localStorage (KTD11). Theme lives in `theme.ts`, which the
// pre-paint script in `index.html` mirrors; the timezone preference is a small external store
// so every timestamp re-renders when it changes.

const TZ_KEY = "timezone";
const listeners = new Set<() => void>();
let timeZone: TimeZonePref | null = null;

function readTimeZone(): TimeZonePref {
  try {
    return localStorage.getItem(TZ_KEY) === "utc" ? "utc" : "local";
  } catch {
    return "local";
  }
}

function getTimeZone(): TimeZonePref {
  timeZone ??= readTimeZone();
  return timeZone;
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function setTimeZonePref(pref: TimeZonePref): void {
  timeZone = pref;
  try {
    localStorage.setItem(TZ_KEY, pref);
  } catch {
    // Storage unavailable (private mode); the choice still applies for this session.
  }
  listeners.forEach((l) => l());
}

export function useTimeZonePref(): TimeZonePref {
  return useSyncExternalStore(subscribe, getTimeZone);
}

/** The shared absolute-time formatter, honoring the timezone preference. */
export function useDateTimeFormatter(): (date: Date | null) => string {
  const pref = useTimeZonePref();
  return (date) => formatDateTime(date, pref);
}

type Stored = boolean | number;
type StoredState<T> = readonly [T, Dispatch<SetStateAction<T>>];

/** Component state persisted in localStorage under `key` (layout choices such as collapsed
 *  panels or widths). A stored value of another type falls back to `initial`. */
export function useStoredState(key: string, initial: boolean): StoredState<boolean>;
export function useStoredState(key: string, initial: number): StoredState<number>;
export function useStoredState<T extends Stored>(key: string, initial: T): StoredState<T> {
  const [value, setValue] = useState<T>(() => {
    try {
      const raw = localStorage.getItem(key);
      const parsed: unknown = raw == null ? null : JSON.parse(raw);
      // Same primitive type as `initial`, which is all T can be.
      return typeof parsed === typeof initial ? (parsed as T) : initial;
    } catch {
      return initial;
    }
  });
  useEffect(() => {
    try {
      localStorage.setItem(key, JSON.stringify(value));
    } catch {
      // Storage unavailable; the choice still applies for this session.
    }
  }, [key, value]);
  return [value, setValue] as const;
}
