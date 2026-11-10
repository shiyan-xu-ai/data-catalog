import { createContext, useContext, useState } from "react";
import { createPortal } from "react-dom";

// A slot under the scrolling page area, beside the left nav, that a page can fill (the table
// page's SQL dock). Pages portal into it so the root layout stays page-agnostic.

const DockContext = createContext<HTMLElement | null>(null);

/** Root-layout side: renders `children` (the page area) and the slot element below it. */
export function BottomDockProvider({ children }: { children: React.ReactNode }) {
  const [slot, setSlot] = useState<HTMLDivElement | null>(null);
  return (
    <DockContext.Provider value={slot}>
      {children}
      <div ref={setSlot} className="shrink-0" />
    </DockContext.Provider>
  );
}

/** Page side: renders `children` into the bottom slot once it is mounted. */
export function BottomDock({ children }: { children: React.ReactNode }) {
  const slot = useContext(DockContext);
  return slot ? createPortal(children, slot) : null;
}
