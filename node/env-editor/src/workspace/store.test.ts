import { describe, expect, test } from "bun:test";
import type { WorkspaceDoc } from "./model";
import {
  createLocalStorageWorkspaceStore,
  createMemoryWorkspaceStore,
  type KeyValueStorage,
} from "./store";

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
