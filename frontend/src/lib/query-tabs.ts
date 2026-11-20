/** One query tab. `id` is also its number in the title; numbers are never reused. */
export interface QueryTab {
  id: number;
  text: string;
}

export interface TabsState {
  tabs: QueryTab[];
  activeId: number;
  /** The number the next opened tab takes. */
  nextId: number;
}

export type TabsAction =
  | { type: "open"; text?: string }
  | { type: "close"; id: number }
  | { type: "activate"; id: number }
  | { type: "setText"; id: number; text: string };

export const tabTitle = (tab: Pick<QueryTab, "id">) => `Query ${tab.id}`;

/** One tab holding `text`. */
export function initialTabs(text = ""): TabsState {
  return { tabs: [{ id: 1, text }], activeId: 1, nextId: 2 };
}

/** Open, close, activate, and edit tabs. Closing the active tab activates the tab that takes
 *  its place (the previous one for the last tab), and closing the only tab opens a fresh one. */
export function tabsReducer(state: TabsState, action: TabsAction): TabsState {
  switch (action.type) {
    case "open": {
      const tab = { id: state.nextId, text: action.text ?? "" };
      return { tabs: [...state.tabs, tab], activeId: tab.id, nextId: tab.id + 1 };
    }
    case "close": {
      const index = state.tabs.findIndex((t) => t.id === action.id);
      if (index < 0) return state;
      const tabs = state.tabs.filter((t) => t.id !== action.id);
      if (tabs.length === 0) return { tabs: [{ id: state.nextId, text: "" }], activeId: state.nextId, nextId: state.nextId + 1 };
      const activeId = state.activeId === action.id ? tabs[Math.min(index, tabs.length - 1)].id : state.activeId;
      return { ...state, tabs, activeId };
    }
    case "activate":
      return state.tabs.some((t) => t.id === action.id) ? { ...state, activeId: action.id } : state;
    case "setText":
      return { ...state, tabs: state.tabs.map((t) => (t.id === action.id ? { ...t, text: action.text } : t)) };
  }
}
