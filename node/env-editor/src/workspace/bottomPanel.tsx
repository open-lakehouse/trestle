// Which tab of the bottom panel is selected, and whether the panel is shown.
// The state lives in a small store (as the workbench's does), so a host can
// open a panel from code it runs outside the shell: a ⌘K command, a header
// action, a menu.
//
// A host that opens panels wraps the shell (and whatever builds its
// contributions) in a `BottomPanelProvider`, and calls
// `useBottomPanel().open(id)`. Without one, the shell keeps a store of its own.

import {
  createContext,
  type ReactNode,
  useContext,
  useMemo,
  useState,
  useSyncExternalStore,
} from "react";

/** The editor's Problems tab. Reserved: a host panel can't take this id. */
export const PROBLEMS_PANEL = "problems";

/** The static Run output tab, shown when the host contributes no panel. */
export const RUN_OUTPUT_PANEL = "output";

/** The bottom panel's state. */
export interface BottomPanelState {
  /** A tab's id: `"problems"`, a host panel's id, or `"output"` (the static
   *  Run output). A view that is no longer listed falls back (see
   *  `activePanelTab`). */
  view: string;
  /** Whether the panel is shown (⌘J toggles it). */
  shown: boolean;
}

/** Holds the bottom panel's state, and tells listeners about changes. */
export interface BottomPanelStore {
  state(): BottomPanelState;
  /** Merge `patch` into the state; listeners are told only on a change. */
  set(patch: Partial<BottomPanelState>): void;
  /** Calls `onChange` after every change; returns an unsubscribe function. */
  subscribe(onChange: () => void): () => void;
}

/** A bottom panel store, starting on Problems and shown by default. */
export function createBottomPanel(
  initial: Partial<BottomPanelState> = {},
): BottomPanelStore {
  let state: BottomPanelState = {
    view: PROBLEMS_PANEL,
    shown: true,
    ...initial,
  };
  const listeners = new Set<() => void>();
  return {
    state: () => state,
    set: (patch) => {
      const next = { ...state, ...patch };
      if (next.view === state.view && next.shown === state.shown) return;
      state = next;
      for (const l of listeners) l();
    },
    subscribe: (onChange) => {
      listeners.add(onChange);
      return () => {
        listeners.delete(onChange);
      };
    },
  };
}

/** The tab to show for a selected view: the selected one while it is listed,
 *  else the first host panel, else Problems. With no host panels the static
 *  Run output (`"output"`) is the one listed tab after Problems. */
export function activePanelTab(
  selected: string,
  panels: readonly { id: string }[],
): string {
  if (selected === PROBLEMS_PANEL) return selected;
  if (panels.some((p) => p.id === selected)) return selected;
  if (panels.length === 0 && selected === RUN_OUTPUT_PANEL) return selected;
  return panels[0]?.id ?? PROBLEMS_PANEL;
}

const BottomPanelContext = createContext<BottomPanelStore | undefined>(
  undefined,
);

export interface BottomPanelProviderProps {
  /** A store to share (e.g. one a host created for its ⌘K commands); a new
   *  one otherwise. */
  panel?: BottomPanelStore;
  children: ReactNode;
}

/** Shares one bottom panel store with the shell and the host code around it. */
export function BottomPanelProvider({
  panel,
  children,
}: BottomPanelProviderProps) {
  const [own] = useState(() => createBottomPanel());
  return (
    <BottomPanelContext.Provider value={panel ?? own}>
      {children}
    </BottomPanelContext.Provider>
  );
}

/** The nearest `BottomPanelProvider`'s store, if any. */
export function useBottomPanelStore(): BottomPanelStore | undefined {
  return useContext(BottomPanelContext);
}

/** The shell's handle on the bottom panel: its state, and how to change it.
 *  Needs a `BottomPanelProvider` above it. */
export function useBottomPanelState(): BottomPanelState & {
  set: (patch: Partial<BottomPanelState>) => void;
  /** Shows the panel if it is hidden, and hides it otherwise. */
  toggle: () => void;
} {
  const store = useContext(BottomPanelContext);
  if (!store) {
    throw new Error("useBottomPanel needs a <BottomPanelProvider> above it");
  }
  const state = useSyncExternalStore(store.subscribe, store.state, store.state);
  const actions = useMemo(
    () => ({
      set: store.set,
      toggle: () => store.set({ shown: !store.state().shown }),
    }),
    [store],
  );
  return { ...state, ...actions };
}

/** The bottom panel's selected tab and whether it is shown, and `open(id)` to
 *  show a tab (a host panel's id, or `"problems"`). Needs a
 *  `BottomPanelProvider` above it. */
export function useBottomPanel(): {
  view: string;
  shown: boolean;
  open: (id: string) => void;
} {
  const { view, shown, set } = useBottomPanelState();
  return {
    view,
    shown,
    open: (id) => set({ view: id, shown: true }),
  };
}
