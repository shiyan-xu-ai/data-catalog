import { createContext, useContext } from "react";

/** Whether the surrounding page or tab is the one on screen. Pages and tabs stay mounted while
 *  hidden to keep their state, so anything that listens globally (keys) or plays (media) must
 *  go quiet while this is false. */
export const ActiveContext = createContext(true);

export function useActive(): boolean {
  return useContext(ActiveContext);
}
