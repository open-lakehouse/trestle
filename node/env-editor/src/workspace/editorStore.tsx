// The shell's open editor tabs (an environment, or one of its revisions or
// comparisons), which one is active, and where each environment's tab is (its
// sub-tab, and the file open under Files). The state lives in a small store, as
// the workbench's and the bottom panel's do, so a host can open an environment's
// tab, on one of its sub-tabs, from code it runs outside the shell: a header
// action, a ⌘K command, a detail that is a button.
//
// A host that opens tabs wraps the shell (and whatever builds its contributions)
// in an `EditorProvider`, and calls `useEditor().openEnvironment(id, { subTab })`.
// Without one, the shell keeps a store of its own.

import {
  createContext,
  type ReactNode,
  useContext,
  useMemo,
  useState,
  useSyncExternalStore,
} from "react";
import {
  closeTab,
  type EditorTab,
  type EnvView,
  INITIAL_VIEW,
  neighbourAfterClose,
  openTab,
  pruneTabs,
  tabKey,
} from "./editors.js";
import { EDITOR_VIEW, useWorkbenchStore } from "./workbench.js";

/** The editor tabs' state. Treat it as read-only: the store replaces it. */
export interface EditorState {
  /** The open tabs, in order. */
  tabs: EditorTab[];
  /** The active tab's key (see `tabKey`); `undefined` when no tab is open. */
  activeKey: string | undefined;
  /** Where each environment's tab is, by environment id. An environment
   *  without an entry is on its diagram. */
  views: Record<string, EnvView>;
}

/** What to show when an environment's tab is opened. */
export interface OpenEnvironmentOptions {
  /** The sub-tab to show: the editor's Diagram, Files or Changes, or a host
   *  tab's id. The store keeps any id; an unknown one falls back to the first
   *  enabled tab, and a disabled one says why (see `resolveSubTab`). */
  subTab?: string;
  /** Open one of the environment's revisions, read-only. */
  revision?: number;
  /** Open a comparison of two of its revisions (`"draft"` is the draft). */
  compare?: EditorTab["compare"];
}

/** Holds the editor tabs' state, and tells listeners about changes. */
export interface EditorStore {
  state(): EditorState;
  /** Opens the environment's tab (or activates it when it is open) and sets its
   *  sub-tab. Listeners are told only on a change. */
  openEnvironment(envId: string, opts?: OpenEnvironmentOptions): void;
  /** Activates the open tab keyed `key`; an unknown key is ignored. */
  activate(key: string): void;
  /** Closes the tab keyed `key`. If it was the active one, its right neighbour
   *  (else its left one) becomes active. */
  close(key: string): void;
  /** Sets where environment `envId`'s tab is. */
  setView(envId: string, view: EnvView): void;
  /** Keeps the tabs of the environments in `envIds` and drops the others. The
   *  first time there is an environment, the first one opens, unless a tab is
   *  open already. */
  syncEnvironments(envIds: readonly string[]): void;
  /** Calls `onChange` after every change; returns an unsubscribe function. */
  subscribe(onChange: () => void): () => void;
}

/** The active tab's key: `active` when that tab is open, else the last tab. */
function settle(state: EditorState): EditorState {
  const last = state.tabs.at(-1);
  const open =
    state.activeKey !== undefined &&
    state.tabs.some((t) => tabKey(t) === state.activeKey);
  return {
    ...state,
    activeKey: open ? state.activeKey : last && tabKey(last),
  };
}

/** An editor tabs store, starting from `initial` (no tabs by default). */
export function createEditor(initial: Partial<EditorState> = {}): EditorStore {
  let state: EditorState = settle({
    tabs: [],
    activeKey: undefined,
    views: {},
    ...initial,
  });
  // Whether the first environment has been opened at start-up, so it isn't
  // reopened after the tabs are closed.
  let started = false;
  const listeners = new Set<() => void>();

  const commit = (patch: Partial<EditorState>) => {
    const next = settle({ ...state, ...patch });
    if (
      next.tabs === state.tabs &&
      next.activeKey === state.activeKey &&
      next.views === state.views
    ) {
      return;
    }
    state = next;
    for (const l of listeners) l();
  };

  return {
    state: () => state,
    openEnvironment: (envId, { subTab, revision, compare } = {}) => {
      const tab: EditorTab = { env: envId, revision, compare };
      const key = tabKey(tab);
      const tabs = state.tabs.some((t) => tabKey(t) === key)
        ? state.tabs
        : openTab(state.tabs, tab);
      const current = state.views[envId] ?? INITIAL_VIEW;
      const views =
        subTab === undefined || current.subTab === subTab
          ? state.views
          : { ...state.views, [envId]: { ...current, subTab } };
      commit({ tabs, activeKey: key, views });
    },
    activate: (key) => {
      if (state.tabs.some((t) => tabKey(t) === key)) commit({ activeKey: key });
    },
    close: (key) => {
      if (!state.tabs.some((t) => tabKey(t) === key)) return;
      commit({
        tabs: closeTab(state.tabs, key),
        activeKey:
          key === state.activeKey
            ? neighbourAfterClose(state.tabs, key)
            : state.activeKey,
      });
    },
    setView: (envId, view) => {
      const current = state.views[envId] ?? INITIAL_VIEW;
      if (
        current.subTab === view.subTab &&
        current.filePath === view.filePath
      ) {
        return;
      }
      commit({ views: { ...state.views, [envId]: view } });
    },
    syncEnvironments: (envIds) => {
      let tabs = pruneTabs(state.tabs, new Set(envIds));
      const first = envIds[0];
      if (!started && first !== undefined) {
        started = true;
        if (tabs.length === 0) tabs = [{ env: first }];
      }
      commit({ tabs });
    },
    subscribe: (onChange) => {
      listeners.add(onChange);
      return () => {
        listeners.delete(onChange);
      };
    },
  };
}

const EditorContext = createContext<EditorStore | undefined>(undefined);

export interface EditorProviderProps {
  /** A store to share (e.g. one a host created for its header actions and
   *  ⌘K commands); a new one otherwise. */
  editor?: EditorStore;
  children: ReactNode;
}

/** Shares one editor tabs store with the shell and the host code around it. */
export function EditorProvider({ editor, children }: EditorProviderProps) {
  const [own] = useState(() => createEditor());
  return (
    <EditorContext.Provider value={editor ?? own}>
      {children}
    </EditorContext.Provider>
  );
}

/** The nearest `EditorProvider`'s store, if any. */
export function useEditorStore(): EditorStore | undefined {
  return useContext(EditorContext);
}

/** The editor tabs, and the actions on them. */
export interface EditorHandle {
  /** The open tabs, in order. Read-only: change them through the actions. */
  tabs: EditorTab[];
  /** The active tab's key, if a tab is open. */
  activeKey: string | undefined;
  /** The active tab, if a tab is open. */
  active: EditorTab | undefined;
  /** The active tab's environment id, if a tab is open. */
  activeEnvId: string | undefined;
  /** Where each environment's tab is, by environment id. */
  views: Readonly<Record<string, EnvView>>;
  /** Opens the environment's tab (a revision or comparison with `opts`),
   *  activates it and sets its sub-tab. Shows the editor tabs over a workbench
   *  view, if one has the main area. */
  openEnvironment: (envId: string, opts?: OpenEnvironmentOptions) => void;
  /** Activates an open tab, showing the editor tabs over a workbench view. */
  activate: (key: string) => void;
  /** Closes an open tab (see `EditorStore.close`). */
  close: (key: string) => void;
  /** Sets where environment `envId`'s tab is. */
  setView: (envId: string, view: EnvView) => void;
  /** Keeps the tabs to the workspace's environments. The shell calls it when
   *  the workspace changes. */
  syncEnvironments: (envIds: readonly string[]) => void;
}

/** The editor tabs and how to change them. Needs an `EditorProvider` above
 *  it. */
export function useEditor(): EditorHandle {
  const store = useContext(EditorContext);
  if (!store) {
    throw new Error("useEditor needs an <EditorProvider> above it");
  }
  const state = useSyncExternalStore(store.subscribe, store.state, store.state);
  const workbench = useWorkbenchStore();
  const actions = useMemo(
    () => ({
      // Opening or activating an editor tab brings the editor tabs back over a
      // workbench view, whoever asks (a header action or a ⌘K command included).
      openEnvironment: (envId: string, opts?: OpenEnvironmentOptions) => {
        store.openEnvironment(envId, opts);
        workbench?.open(EDITOR_VIEW);
      },
      activate: (key: string) => {
        store.activate(key);
        workbench?.open(EDITOR_VIEW);
      },
      close: store.close,
      setView: store.setView,
      syncEnvironments: store.syncEnvironments,
    }),
    [store, workbench],
  );
  const active = state.tabs.find((t) => tabKey(t) === state.activeKey);
  return {
    tabs: state.tabs,
    activeKey: state.activeKey,
    active,
    activeEnvId: active?.env,
    views: state.views,
    ...actions,
  };
}
