// Which view has the shell's main area: the editor tabs (`"editor"`), or one
// of a host's workbench views (`EditorContributions.workbenchViews`). The state
// lives in a small store, so a host can switch views from code it runs
// outside the shell: a header action, a ⌘K command, a menu.
//
// A host that switches views wraps the shell (and whatever builds its
// contributions) in a `WorkbenchProvider`, and calls `useWorkbench().open(id)`.
// Without one, the shell keeps a store of its own.

import { cn } from "@open-lakehouse/ui-kit";
import {
  createContext,
  type ReactNode,
  useContext,
  useState,
  useSyncExternalStore,
} from "react";
import type { WorkbenchViewContribution } from "./contributions.js";

/** The editor tabs' id among the workbench's views. */
export const EDITOR_VIEW = "editor";

/** Which view has the main area, and how to switch. */
export interface WorkbenchStore {
  /** `"editor"`, or a workbench view's id. */
  active(): string;
  /** Show workbench view `id`, or the editor tabs (`"editor"`). */
  open(id: string): void;
  /** Calls `onChange` after every switch; returns an unsubscribe function. */
  subscribe(onChange: () => void): () => void;
}

/** A workbench store, starting on `initial` (the editor tabs by default). */
export function createWorkbench(initial: string = EDITOR_VIEW): WorkbenchStore {
  let active = initial;
  const listeners = new Set<() => void>();
  return {
    active: () => active,
    open: (id) => {
      if (id === active) return;
      active = id;
      for (const l of listeners) l();
    },
    subscribe: (onChange) => {
      listeners.add(onChange);
      return () => listeners.delete(onChange);
    },
  };
}

const WorkbenchContext = createContext<WorkbenchStore | undefined>(undefined);

export interface WorkbenchProviderProps {
  /** A store to share (e.g. one a test or a host created); a new one
   *  otherwise. */
  workbench?: WorkbenchStore;
  children: ReactNode;
}

/** Shares one workbench store with the shell and the host code around it. */
export function WorkbenchProvider({
  workbench,
  children,
}: WorkbenchProviderProps) {
  const [own] = useState(createWorkbench);
  return (
    <WorkbenchContext.Provider value={workbench ?? own}>
      {children}
    </WorkbenchContext.Provider>
  );
}

/** The nearest `WorkbenchProvider`'s store, if any. */
export function useWorkbenchStore(): WorkbenchStore | undefined {
  return useContext(WorkbenchContext);
}

/** Which view has the main area (`"editor"` or a workbench view's id), and
 *  `open(id)` to switch. Needs a `WorkbenchProvider` above it. */
export function useWorkbench(): {
  active: string;
  open: (id: string) => void;
} {
  const store = useContext(WorkbenchContext);
  if (!store) {
    throw new Error("useWorkbench needs a <WorkbenchProvider> above it");
  }
  const active = useSyncExternalStore(
    store.subscribe,
    store.active,
    store.active,
  );
  return { active, open: store.open };
}

/** The shell's main area: the editor tabs (`children`), and the active
 *  workbench view over them. The editor tabs stay mounted while hidden, so
 *  the diagram, its undo history and the viewers survive switching back. */
export function WorkbenchArea({
  views = [],
  active,
  children,
}: {
  views?: readonly WorkbenchViewContribution[];
  /** `"editor"`, or a workbench view's id; an unknown id is the editor. */
  active: string;
  children: ReactNode;
}) {
  const view = views.find((v) => v.id === active);
  return (
    <>
      <div
        data-workbench-editor=""
        hidden={!!view}
        className={cn("min-h-0 flex-1 flex-col", view ? "hidden" : "flex")}
      >
        {children}
      </div>
      {view && (
        <section
          aria-label={view.label}
          data-workbench-view={view.id}
          className="min-h-0 flex-1 overflow-auto"
        >
          {view.render()}
        </section>
      )}
    </>
  );
}
