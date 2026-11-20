import type { PointerEvent as ReactPointerEvent } from "react";

/** Captures the pointer on the event's element and calls `onMove` until the drag ends. */
export function dragPointer(event: ReactPointerEvent<HTMLElement>, onMove: (e: PointerEvent) => void) {
  event.preventDefault();
  const handle = event.currentTarget;
  handle.setPointerCapture(event.pointerId);
  const onEnd = () => {
    handle.removeEventListener("pointermove", onMove);
    handle.removeEventListener("pointerup", onEnd);
    handle.removeEventListener("pointercancel", onEnd);
  };
  handle.addEventListener("pointermove", onMove);
  handle.addEventListener("pointerup", onEnd);
  handle.addEventListener("pointercancel", onEnd);
}
