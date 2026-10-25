// The workspace the shell edits, loaded from and saved to the injected
// `WorkspaceStore`. Every change is saved as it happens; a change made
// elsewhere (another tab, or a store watching files) reloads it.

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
} from "react";
import type { Selection } from "../types";
import { type EnvDoc, nameProblem, nextSlot, type WorkspaceDoc } from "./model";
import type { WorkspaceStore } from "./store";

/** The workspace a store starts with when it holds none: one `dev` env. */
export function starterWorkspace(): WorkspaceDoc {
  return {
    name: "local",
    mesh: false,
    envs: [{ id: newId(), name: "dev", slot: 0 }],
  };
}

function newId(): string {
  return (
    globalThis.crypto?.randomUUID?.() ?? Math.random().toString(36).slice(2)
  );
}

export interface WorkspaceState {
  /** The workspace; `undefined` while it loads. */
  workspace: WorkspaceDoc | undefined;
  /** Loading or saving failed. */
  error: string | null;
  /** Add an environment on the next free slot; returns it, or the reason it
   *  can't be added. */
  addEnv: (name: string, selection?: Selection) => EnvDoc | string;
  /** Add a copy of `id`'s environment (its selection) named `name`. */
  duplicateEnv: (id: string, name: string) => EnvDoc | string;
  /** Rename an environment; returns the reason when `name` can't be used. */
  renameEnv: (id: string, name: string) => string | undefined;
  /** Drop an environment, freeing its slot. */
  removeEnv: (id: string) => void;
  /** Record an environment's edited selection. */
  setSelection: (id: string, selection: Selection) => void;
  /** Turn the gateways' shared network on or off. */
  setMesh: (mesh: boolean) => void;
  /** Rename the workspace; returns the reason when `name` can't be used. */
  renameWorkspace: (name: string) => string | undefined;
}

const WorkspaceContext = createContext<WorkspaceState | null>(null);

/** Load the workspace from `store` and keep it saved there. */
export function useWorkspaceState(store: WorkspaceStore): WorkspaceState {
  const [workspace, setWorkspace] = useState<WorkspaceDoc>();
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    const load = () =>
      store
        .load()
        .then((loaded) => {
          if (cancelled) return;
          if (loaded) {
            setWorkspace(loaded);
          } else {
            const starter = starterWorkspace();
            setWorkspace(starter);
            void store.save(starter);
          }
        })
        .catch((e: unknown) => {
          if (!cancelled) setError(message(e));
        });
    void load();
    const unsubscribe = store.subscribe?.(() => void load());
    return () => {
      cancelled = true;
      unsubscribe?.();
    };
  }, [store]);

  /** Apply `change` to the loaded workspace and save the result. */
  const update = useCallback(
    (change: (w: WorkspaceDoc) => WorkspaceDoc) => {
      setWorkspace((current) => {
        if (!current) return current;
        const next = change(current);
        if (next !== current) {
          store.save(next).catch((e: unknown) => setError(message(e)));
        }
        return next;
      });
    },
    [store],
  );

  const add = useCallback(
    (name: string, selection?: Selection): EnvDoc | string => {
      if (!workspace) return "The workspace is still loading.";
      const problem = nameProblem(workspace, name);
      if (problem) return problem;
      const slot = nextSlot(workspace);
      if (slot === undefined) return "Every slot is taken.";
      const env: EnvDoc = { id: newId(), name, slot, selection };
      update((w) => ({ ...w, envs: [...w.envs, env] }));
      return env;
    },
    [workspace, update],
  );

  const duplicateEnv = useCallback(
    (id: string, name: string) =>
      add(name, workspace?.envs.find((e) => e.id === id)?.selection),
    [add, workspace],
  );

  const renameEnv = useCallback(
    (id: string, name: string) => {
      if (!workspace) return "The workspace is still loading.";
      const problem = nameProblem(workspace, name, id);
      if (problem) return problem;
      update((w) => ({
        ...w,
        envs: w.envs.map((e) => (e.id === id ? { ...e, name } : e)),
      }));
      return undefined;
    },
    [workspace, update],
  );

  const removeEnv = useCallback(
    (id: string) =>
      update((w) => ({ ...w, envs: w.envs.filter((e) => e.id !== id) })),
    [update],
  );

  const setSelection = useCallback(
    (id: string, selection: Selection) => {
      setWorkspace((current) => {
        const env = current?.envs.find((e) => e.id === id);
        if (!current || !env) return current;
        const next = { ...env, selection };
        store.saveEnv(next).catch((e: unknown) => setError(message(e)));
        return {
          ...current,
          envs: current.envs.map((e) => (e.id === id ? next : e)),
        };
      });
    },
    [store],
  );

  const setMesh = useCallback(
    (mesh: boolean) => update((w) => (w.mesh === mesh ? w : { ...w, mesh })),
    [update],
  );

  const renameWorkspace = useCallback(
    (name: string) => {
      if (!workspace) return "The workspace is still loading.";
      const problem = nameProblem({ ...workspace, envs: [] }, name);
      if (problem) return problem;
      update((w) => ({ ...w, name }));
      return undefined;
    },
    [workspace, update],
  );

  return useMemo(
    () => ({
      workspace,
      error,
      addEnv: add,
      duplicateEnv,
      renameEnv,
      removeEnv,
      setSelection,
      setMesh,
      renameWorkspace,
    }),
    [
      workspace,
      error,
      add,
      duplicateEnv,
      renameEnv,
      removeEnv,
      setSelection,
      setMesh,
      renameWorkspace,
    ],
  );
}

export interface WorkspaceProviderProps {
  store: WorkspaceStore;
  children: React.ReactNode;
}

/** Provide the workspace loaded from `store` to {@link useWorkspace}. */
export function WorkspaceProvider({ store, children }: WorkspaceProviderProps) {
  const state = useWorkspaceState(store);
  return (
    <WorkspaceContext.Provider value={state}>
      {children}
    </WorkspaceContext.Provider>
  );
}

/** The workspace from the nearest {@link WorkspaceProvider}. */
export function useWorkspace(): WorkspaceState {
  const state = useContext(WorkspaceContext);
  if (!state) {
    throw new Error("useWorkspace must be used within a <WorkspaceProvider>");
  }
  return state;
}

function message(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}
