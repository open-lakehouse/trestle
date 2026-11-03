import { HostProvider } from "@open-lakehouse/host/react";
import { createFakeHost } from "@open-lakehouse/host/testing";
import {
  FIXTURE_CATALOG,
  FIXTURE_PLAN,
} from "@open-lakehouse/stack-wasm/fixtures";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { PlannerProvider } from "../planner.js";
import type { Planner, Selection } from "../types.js";
import type { WorkspaceDoc } from "./model.js";
import {
  createMemoryWorkspaceStore,
  supportsRevisions,
  type WorkspaceStore,
} from "./store.js";
import { WorkspaceProvider } from "./useWorkspace.js";
import { WorkspaceShell } from "./WorkspaceShell.js";

// The fixture planner: the captured catalog, and one representative plan.
const planner: Planner = {
  catalog: async () => FIXTURE_CATALOG,
  plan: async () => FIXTURE_PLAN,
};

const selection: Selection = FIXTURE_CATALOG.default_selection;
const workspace: WorkspaceDoc = {
  name: "local",
  mesh: false,
  envs: [
    { id: "dev", name: "dev", slot: 0, selection },
    { id: "staging", name: "staging", slot: 1 },
  ],
};

/** A store with two committed revisions of `dev`, and a changed draft. */
async function storeWithHistory(): Promise<WorkspaceStore> {
  const store = createMemoryWorkspaceStore(workspace, { revisions: true });
  if (!supportsRevisions(store)) throw new Error("expected revisions");
  await store.commit("dev", "first cut");
  const [first, ...rest] = selection.modules;
  await store.saveEnv({
    id: "dev",
    name: "dev",
    slot: 0,
    head: 1,
    selection: { ...selection, modules: rest },
  });
  await store.commit("dev", `drop ${first ?? "a module"}`);
  await store.saveEnv({ id: "dev", name: "dev", slot: 0, head: 2, selection });
  return store;
}

function Shell({ store }: { store: WorkspaceStore }) {
  return (
    <HostProvider host={createFakeHost({ kind: "desktop" })}>
      <PlannerProvider planner={planner}>
        <WorkspaceProvider store={store}>
          <div className="h-screen">
            <WorkspaceShell catalog={FIXTURE_CATALOG} />
          </div>
        </WorkspaceProvider>
      </PlannerProvider>
    </HostProvider>
  );
}

const meta: Meta<typeof Shell> = {
  title: "WorkspaceShell",
  component: Shell,
  parameters: { layout: "fullscreen" },
};

export default meta;

type Story = StoryObj<typeof Shell>;

/** A store with drafts only (like browser storage): no history view. */
export const DraftsOnly: Story = {
  render: () => <Shell store={createMemoryWorkspaceStore(workspace)} />,
};

/** A store that keeps revisions: the History view lists, opens and compares
 *  them, and commits the draft. */
export const WithRevisions: Story = {
  loaders: [async () => ({ store: await storeWithHistory() })],
  render: (_args, { loaded }) => <Shell store={loaded.store} />,
};
