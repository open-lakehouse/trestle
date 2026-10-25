// The workspace store seam: where the editor keeps its workspace. Like the
// `Planner`, a host injects it, so the same shell runs against browser storage
// today and, later, a folder opened with the File System Access API or a local
// `trestle workspace serve` server. Every method is async for those.

import type { EnvDoc, WorkspaceDoc } from "./model";

/** Where a workspace lives. */
export interface WorkspaceStore {
  /** The workspace, or `null` when there is none yet. */
  load(): Promise<WorkspaceDoc | null>;
  /** Save the workspace's own settings (name, mesh) and its member list. */
  save(workspace: WorkspaceDoc): Promise<void>;
  /** Save one member (its selection changed); cheaper than {@link save}. */
  saveEnv(env: EnvDoc): Promise<void>;
  /** Watch for changes made elsewhere (another tab, the CLI on disk). Returns
   *  an unsubscribe function. Optional: a store nothing else writes to omits it. */
  subscribe?(onChange: () => void): () => void;
}

/** The subset of the Web Storage API the browser store needs. */
export type KeyValueStorage = Pick<Storage, "getItem" | "setItem">;

/** The storage key the browser store uses by default. */
export const DEFAULT_WORKSPACE_KEY = "trestle.workspace.v1";

/**
 * A {@link WorkspaceStore} in browser storage (`localStorage` by default),
 * the whole workspace as one JSON value under `key`. Other tabs' writes arrive
 * through `subscribe` (the `storage` event).
 */
export function createLocalStorageWorkspaceStore({
  storage = globalThis.localStorage,
  key = DEFAULT_WORKSPACE_KEY,
}: {
  storage?: KeyValueStorage;
  key?: string;
} = {}): WorkspaceStore {
  const read = (): WorkspaceDoc | null => {
    const raw = storage.getItem(key);
    if (!raw) return null;
    try {
      return JSON.parse(raw) as WorkspaceDoc;
    } catch {
      return null;
    }
  };
  const write = (workspace: WorkspaceDoc) =>
    storage.setItem(key, JSON.stringify(workspace));
  return {
    async load() {
      return read();
    },
    async save(workspace) {
      write(workspace);
    },
    async saveEnv(env) {
      const workspace = read();
      if (!workspace) return;
      write({
        ...workspace,
        envs: workspace.envs.map((e) => (e.id === env.id ? env : e)),
      });
    },
    subscribe(onChange) {
      if (typeof window === "undefined") return () => {};
      const listener = (e: StorageEvent) => {
        if (e.key === key) onChange();
      };
      window.addEventListener("storage", listener);
      return () => window.removeEventListener("storage", listener);
    },
  };
}

/** An in-memory {@link WorkspaceStore}, for tests and stories. */
export function createMemoryWorkspaceStore(
  initial: WorkspaceDoc | null = null,
): WorkspaceStore {
  const values = new Map<string, string>();
  const store = createLocalStorageWorkspaceStore({
    storage: {
      getItem: (k) => values.get(k) ?? null,
      setItem: (k, v) => {
        values.set(k, v);
      },
    },
  });
  if (initial) void store.save(initial);
  return { load: store.load, save: store.save, saveEnv: store.saveEnv };
}
