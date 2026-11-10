import { useSyncExternalStore } from "react";
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
