import { describe, expect, test } from "bun:test";
import { renderToString } from "react-dom/server";
import { PlannerProvider } from "../planner.js";
import { useEnvironment } from "../state/useEnvironment.js";
import type { Selection } from "../types.js";
import type { WorkspaceDoc } from "./model.js";
import {
  createLocalStorageWorkspaceStore,
  createMemoryWorkspaceStore,
  type WorkspaceStore,
} from "./store.js";
import {
  migrateWorkspace,
  useWorkspace,
  WorkspaceProvider,
} from "./useWorkspace.js";

function Probe() {
  const { revisions } = useWorkspace();
  return <span>{revisions ? "revisions" : "drafts-only"}</span>;
}

const render = (store: WorkspaceStore) =>
  renderToString(
    <WorkspaceProvider store={store}>
      <Probe />
    </WorkspaceProvider>,
  );

describe("useWorkspaceState revisions", () => {
  test("stores without revisions offer none, so the shell hides them", () => {
    const storage = { getItem: () => null, setItem: () => {} };
    expect(render(createLocalStorageWorkspaceStore({ storage }))).toContain(
      "drafts-only",
    );
    expect(render(createMemoryWorkspaceStore())).toContain("drafts-only");
  });

  test("a store with revisions offers them", () => {
    expect(
      render(createMemoryWorkspaceStore(null, { revisions: true })),
    ).toContain("revisions");
  });
});

describe("selections saved before a module was renamed", () => {
  const legacy: Selection = {
    modules: ["unity-catalog", "mlflow"],
    capabilities: [],
    knob_overrides: { "unity-catalog": { bucket: "lake" } },
  };
  const doc: WorkspaceDoc = {
    name: "demo",
    mesh: false,
    envs: [
      { id: "a", name: "dev", slot: 0, selection: legacy },
      { id: "b", name: "prod", slot: 1 },
    ],
  };

  test("a workspace adopts the new ids, and is unchanged when current", () => {
    const migrated = migrateWorkspace(doc);
    expect(migrated.envs[0]?.selection?.modules).toEqual([
      "unity-catalog-oss",
      "mlflow",
    ]);
    expect(migrated.envs[0]?.selection?.knob_overrides).toEqual({
      "unity-catalog-oss": { bucket: "lake" },
    });
    expect(migrated.envs[1]).toBe(doc.envs[1]);
    expect(migrateWorkspace(migrated)).toBe(migrated);
  });

  test("the editor edits a stored legacy selection under the new id", () => {
    function Probe() {
      const env = useEnvironment(undefined, { initialSelection: legacy });
      return <span>{env.selection.modules.join(",")}</span>;
    }
    const planner = {
      catalog: () => new Promise<never>(() => {}),
      plan: () => new Promise<never>(() => {}),
    };
    const html = renderToString(
      <PlannerProvider planner={planner}>
        <Probe />
      </PlannerProvider>,
    );
    expect(html).toContain("unity-catalog-oss,mlflow");
  });

  test("a committed revision loads under the new ids", async () => {
    const store = createMemoryWorkspaceStore(doc, { revisions: true });
    await store.commit?.("a");
    let revisions: ReturnType<typeof useWorkspace>["revisions"];
    function Grab() {
      revisions = useWorkspace().revisions;
      return null;
    }
    renderToString(
      <WorkspaceProvider store={store}>
        <Grab />
      </WorkspaceProvider>,
    );
    const loaded = await revisions?.load("a", 1);
    expect(loaded?.selection.modules).toEqual(["unity-catalog-oss", "mlflow"]);
  });
});
