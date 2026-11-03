// The workspace store seam: where the editor keeps its workspace. Like the
// `Planner`, a host injects it, so the same shell runs against browser storage,
// a folder, a local `trestle workspace serve` server or the desktop's shared
// state store. Every method is async for those.
//
// A store may also keep revisions: numbered, immutable snapshots of an
// environment, as `olai-stack-state` does. The editor always edits the draft
// (`EnvDoc.selection`); committing it makes the next revision. A store without
// the revision methods (browser storage) has drafts only, and the shell hides
// everything revision-related.

import type { Selection } from "../types.js";
import type { EnvDoc, WorkspaceDoc } from "./model.js";

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

  /** The revisions of environment `envId`, newest first. Optional, with
   *  {@link loadRevision} and {@link commit}: see {@link supportsRevisions}. */
  listRevisions?(envId: string): Promise<RevisionSummary[]>;
  /** Revision `number` of environment `envId`, or `null` when it has none. */
  loadRevision?(envId: string, number: number): Promise<RevisionDoc | null>;
  /**
   * Commit environment `envId`'s draft as its next revision and return it. The
   * env's `head` becomes the new number; the draft stays as it was.
   */
  commit?(envId: string, message?: string): Promise<RevisionSummary>;
}

/** A revision in a list: enough to label it and tell two apart. Mirrors
 *  `olai-stack-state`'s `Revision`. */
export interface RevisionSummary {
  /** 1 for an environment's first revision, counting up. */
  number: number;
  /** A digest of its content; equal digests mean equal revisions. */
  digest: string;
  /** When it was committed (ISO 8601), where the store records it. */
  createdAt?: string;
  /** Why it was committed, where the committer said. */
  message?: string;
}

/** A revision's content. */
export interface RevisionDoc extends RevisionSummary {
  /** The selection it froze. */
  selection: Selection;
  /** The manifest as stored (`env.toml`), where the store keeps it. */
  manifestToml?: string;
}

/** A store that keeps revisions. */
export type RevisionStore = WorkspaceStore &
  Required<Pick<WorkspaceStore, "listRevisions" | "loadRevision" | "commit">>;

/** Whether `store` keeps revisions: it has all three revision methods. */
export function supportsRevisions(
  store: WorkspaceStore,
): store is RevisionStore {
  return (
    typeof store.listRevisions === "function" &&
    typeof store.loadRevision === "function" &&
    typeof store.commit === "function"
  );
}

/** JSON with object keys sorted, so equal values serialize equally. */
function canonicalJson(value: unknown): string {
  return JSON.stringify(value, (_key, v: unknown) =>
    v && typeof v === "object" && !Array.isArray(v)
      ? Object.fromEntries(
          Object.entries(v as Record<string, unknown>).sort(([a], [b]) =>
            a < b ? -1 : a > b ? 1 : 0,
          ),
        )
      : v,
  );
}

/** The SHA-256 (hex) of `selection`'s canonical JSON. */
export async function selectionDigest(selection: Selection): Promise<string> {
  const bytes = new TextEncoder().encode(canonicalJson(selection));
  const hash = await globalThis.crypto.subtle.digest("SHA-256", bytes);
  return [...new Uint8Array(hash)]
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
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

/** Options for {@link createMemoryWorkspaceStore}. */
export interface MemoryStoreOptions {
  /** Keep revisions (commit, list, load) in memory too. Off by default. */
  revisions?: boolean;
  /** The clock commits are stamped with; defaults to now. */
  now?: () => Date;
}

/** An in-memory {@link WorkspaceStore}, for tests and stories. */
export function createMemoryWorkspaceStore(
  initial: WorkspaceDoc | null = null,
  { revisions = false, now = () => new Date() }: MemoryStoreOptions = {},
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
  const base: WorkspaceStore = {
    load: store.load,
    save: store.save,
    saveEnv: store.saveEnv,
  };
  if (!revisions) return base;

  // Kept as JSON, so a caller can't mutate a committed revision.
  const history = new Map<string, string[]>();
  const revisionsOf = (envId: string): RevisionDoc[] =>
    (history.get(envId) ?? []).map((r) => JSON.parse(r) as RevisionDoc);

  return {
    ...base,
    async listRevisions(envId) {
      return revisionsOf(envId)
        .reverse()
        .map(({ selection: _s, manifestToml: _m, ...summary }) => summary);
    },
    async loadRevision(envId, number) {
      return revisionsOf(envId).find((r) => r.number === number) ?? null;
    },
    async commit(envId, message) {
      const workspace = await store.load();
      const env = workspace?.envs.find((e) => e.id === envId);
      if (!workspace || !env) {
        throw new Error(`no environment ${envId} to commit`);
      }
      if (!env.selection) {
        throw new Error(`${env.name} has no edits to commit yet`);
      }
      const committed = history.get(envId) ?? [];
      const revision: RevisionDoc = {
        number: committed.length + 1,
        digest: await selectionDigest(env.selection),
        createdAt: now().toISOString(),
        ...(message ? { message } : {}),
        selection: env.selection,
      };
      history.set(envId, [...committed, JSON.stringify(revision)]);
      await store.saveEnv({ ...env, head: revision.number });
      const { selection: _s, ...summary } = revision;
      return summary;
    },
  };
}
