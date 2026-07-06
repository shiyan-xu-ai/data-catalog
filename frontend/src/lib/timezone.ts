import { useSyncExternalStore } from "react";

export type TzMode = "local" | "utc";

const KEY = "tz";

function read(): TzMode {
  try {
    return localStorage.getItem(KEY) === "utc" ? "utc" : "local";
  } catch {
    return "local";
  }
}

let mode: TzMode = read();
const subscribers = new Set<() => void>();

function notify(): void {
  for (const cb of subscribers) cb();
}

export function getTz(): TzMode {
  return mode;
}

export function setTz(m: TzMode): void {
  mode = m;
  try {
    localStorage.setItem(KEY, m);
  } catch {
    // ignore — storage may be unavailable
  }
  notify();
}

export function toggleTz(): void {
  setTz(mode === "utc" ? "local" : "utc");
}

export function subscribe(cb: () => void): () => void {
  subscribers.add(cb);
  return () => {
    subscribers.delete(cb);
  };
}

export function useTz(): { mode: TzMode; toggle: () => void } {
  const current = useSyncExternalStore(subscribe, getTz, getTz);
  return { mode: current, toggle: toggleTz };
}
