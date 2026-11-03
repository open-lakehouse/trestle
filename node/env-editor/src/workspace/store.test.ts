import { describe, expect, test } from "bun:test";
import type { Selection } from "../types.js";
import type { WorkspaceDoc } from "./model.js";
import {
  createLocalStorageWorkspaceStore,
  createMemoryWorkspaceStore,
  type KeyValueStorage,
  selectionDigest,
  supportsRevisions,
  type WorkspaceStore,
} from "./store.js";

const workspace: WorkspaceDoc = {
  name: "demo",
  mesh: false,
  envs: [
    { id: "a", name: "dev", slot: 0 },
    { id: "b", name: "prod", slot: 1 },
  ],
};

function fakeStorage(): KeyValueStorage & { values: Map<string, string> } {
  const values = new Map<string, string>();
  return {
    values,
    getItem: (k) => values.get(k) ?? null,
    setItem: (k, v) => {
      values.set(k, v);
    },
  };
}

describe("workspace stores", () => {
  test("browser storage round-trips the workspace under its key", async () => {
    const storage = fakeStorage();
    const store = createLocalStorageWorkspaceStore({ storage, key: "k" });
    expect(await store.load()).toBeNull();
    await store.save(workspace);
    expect([...storage.values.keys()]).toEqual(["k"]);
    expect(await store.load()).toEqual(workspace);
  });

  test("saveEnv replaces only that member", async () => {
    const store = createMemoryWorkspaceStore(workspace);
    const selection = {
      modules: ["postgres"],
      capabilities: [],
      knob_overrides: {},
    };
    await store.saveEnv({ id: "b", name: "prod", slot: 1, selection });
    const loaded = await store.load();
    expect(loaded?.envs[0]).toEqual(workspace.envs[0]);
    expect(loaded?.envs[1]?.selection).toEqual(selection);
  });

  test("an unreadable value loads as no workspace", async () => {
    const storage = fakeStorage();
    storage.setItem("k", "{not json");
    const store = createLocalStorageWorkspaceStore({ storage, key: "k" });
    expect(await store.load()).toBeNull();
  });
});

/** Every kind of store the editor ships, and whether it keeps revisions. */
const kinds: [string, (initial: WorkspaceDoc) => WorkspaceStore, boolean][] = [
  [
    "browser storage",
    (initial) => {
      const store = createLocalStorageWorkspaceStore({
        storage: fakeStorage(),
      });
      void store.save(initial);
      return store;
    },
    false,
  ],
  ["memory", (initial) => createMemoryWorkspaceStore(initial), false],
  [
    "memory with revisions",
    (initial) => createMemoryWorkspaceStore(initial, { revisions: true }),
    true,
  ],
];

const pg: Selection = {
  modules: ["postgres"],
  capabilities: [],
  knob_overrides: {},
};

describe.each(kinds)("%s", (_name, make, revisions) => {
  test("load, save and saveEnv work the same", async () => {
    const store = make(workspace);
    expect(await store.load()).toEqual(workspace);
    await store.saveEnv({ id: "a", name: "dev", slot: 0, selection: pg });
    expect((await store.load())?.envs[0]?.selection).toEqual(pg);
    await store.save({ ...workspace, mesh: true });
    expect((await store.load())?.mesh).toBe(true);
  });

  test(`supportsRevisions is ${revisions}`, () => {
    expect(supportsRevisions(make(workspace))).toBe(revisions);
  });
});

describe("a store with revisions", () => {
  const at = new Date("2026-11-03T12:00:00Z");
  const make = () =>
    createMemoryWorkspaceStore(
      {
        ...workspace,
        envs: [{ id: "a", name: "dev", slot: 0, selection: pg }],
      },
      { revisions: true, now: () => at },
    );

  test("commit numbers revisions per env and moves head", async () => {
    const store = make();
    if (!supportsRevisions(store)) throw new Error("expected revisions");
    expect(await store.listRevisions("a")).toEqual([]);
    const first = await store.commit("a", "initial");
    expect(first).toEqual({
      number: 1,
      digest: await selectionDigest(pg),
      createdAt: at.toISOString(),
      message: "initial",
    });
    expect((await store.load())?.envs[0]?.head).toBe(1);

    const next: Selection = { ...pg, modules: ["postgres", "rustfs"] };
    await store.saveEnv({
      id: "a",
      name: "dev",
      slot: 0,
      head: 1,
      selection: next,
    });
    const second = await store.commit("a");
    expect(second.number).toBe(2);
    expect(second.message).toBeUndefined();
    expect(second.digest).not.toBe(first.digest);
    expect((await store.load())?.envs[0]?.head).toBe(2);
    expect((await store.listRevisions("a")).map((r) => r.number)).toEqual([
      2, 1,
    ]);
  });

  test("a revision keeps what was committed, whatever the draft does", async () => {
    const store = make();
    if (!supportsRevisions(store)) throw new Error("expected revisions");
    await store.commit("a");
    await store.saveEnv({
      id: "a",
      name: "dev",
      slot: 0,
      selection: { ...pg, modules: [] },
    });
    const loaded = await store.loadRevision("a", 1);
    expect(loaded?.selection).toEqual(pg);
    // Mutating what was handed out doesn't reach the stored revision.
    loaded?.selection.modules.push("mutated");
    expect((await store.loadRevision("a", 1))?.selection).toEqual(pg);
    expect(await store.loadRevision("a", 2)).toBeNull();
  });

  test("committing nothing, or an unknown env, fails", async () => {
    const store = createMemoryWorkspaceStore(workspace, { revisions: true });
    if (!supportsRevisions(store)) throw new Error("expected revisions");
    await expect(store.commit("a")).rejects.toThrow(/no edits/);
    await expect(store.commit("nope")).rejects.toThrow(/no environment/);
  });

  test("the digest ignores key order", async () => {
    expect(
      await selectionDigest({
        knob_overrides: {},
        capabilities: [],
        modules: ["x"],
      }),
    ).toBe(
      await selectionDigest({
        modules: ["x"],
        capabilities: [],
        knob_overrides: {},
      }),
    );
  });
});
