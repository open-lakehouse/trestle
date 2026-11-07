import { HostProvider } from "@open-lakehouse/host/react";
import { createFakeHost } from "@open-lakehouse/host/testing";
import {
  FIXTURE_CATALOG,
  FIXTURE_PLAN,
} from "@open-lakehouse/stack-wasm/fixtures";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Activity, Play, Square, Stethoscope, Wrench } from "lucide-react";
import { PlannerProvider } from "../planner.js";
import type { Planner, Selection } from "../types.js";
import {
  ContributionsProvider,
  type EditorContributions,
  type RunState,
} from "./contributions.js";
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

function Shell({
  store,
  contributions,
}: {
  store: WorkspaceStore;
  contributions?: EditorContributions;
}) {
  return (
    <HostProvider host={createFakeHost({ kind: "desktop" })}>
      <PlannerProvider planner={planner}>
        <WorkspaceProvider store={store}>
          <ContributionsProvider contributions={contributions}>
            <div className="h-screen">
              <WorkspaceShell catalog={FIXTURE_CATALOG} />
            </div>
          </ContributionsProvider>
        </WorkspaceProvider>
      </PlannerProvider>
    </HostProvider>
  );
}

/**
 * A pretend runtime for the contributions story: `dev` runs locally in slot 0,
 * `staging` on a remote branch. Start and Stop step through starting/stopping
 * on a timer, and notify through `subscribe` like a query cache would.
 */
function fakeRuntime(): EditorContributions {
  const state: Record<string, RunState> = {
    dev: "stopped",
    staging: "running",
  };
  const placement: Record<string, string> = {
    dev: "local slot 0",
    staging: "branch preview/staging",
  };
  const listeners = new Set<() => void>();
  const set = (id: string, next: RunState) => {
    state[id] = next;
    for (const l of listeners) l();
  };
  const transition = (id: string, via: RunState, to: RunState) => {
    set(id, via);
    setTimeout(() => set(id, to), 1200);
  };
  const running = (id: string) => state[id] === "running";
  const notRunning = (id: string) => `${id} isn't running`;
  const view = (title: string, body: string) => () => (
    <div className="p-3 text-sm">
      <div className="mb-2 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
        {title}
      </div>
      <p className="text-muted-foreground">{body}</p>
    </div>
  );
  return {
    sideViews: [
      {
        id: "doctor",
        label: "Doctor",
        icon: <Stethoscope />,
        render: view("Doctor", "Docker is running. trestle 0.9 is on PATH."),
      },
      {
        id: "app-settings",
        label: "App settings",
        icon: <Wrench />,
        align: "end",
        render: view("App settings", "Theme, telemetry, update channel."),
      },
    ],
    environment: {
      subscribe: (onChange) => {
        listeners.add(onChange);
        return () => listeners.delete(onChange);
      },
      status: (id) =>
        state[id] && {
          state: state[id],
          revision: id === "dev" ? 2 : 1,
          placement: placement[id],
          detail: running(id) ? "all services healthy" : undefined,
        },
      actions: (id) =>
        running(id)
          ? [
              {
                id: "stop",
                label: "Stop",
                icon: <Square />,
                run: () => transition(id, "stopping", "stopped"),
              },
            ]
          : [
              {
                id: "start",
                label: "Start",
                icon: <Play />,
                variant: "primary",
                enabled: state[id] === "stopped",
                reason: `${id} is ${state[id]}`,
                run: () => transition(id, "starting", "running"),
              },
            ],
      tabs: (id) =>
        ["Catalog", "Lineage", "Logs"].map((label) => ({
          id: label.toLowerCase(),
          label,
          enabled: running(id),
          reason: notRunning(id),
          render: view(label, `${label} of ${id}, from the host.`),
        })),
      nodeStatus: (id, moduleId) =>
        state[id] === "stopped"
          ? undefined
          : moduleId === "mlflow" && running(id)
            ? { state: "degraded", label: "unhealthy", detail: "2 restarts" }
            : { state: state[id] ?? "unknown" },
    },
    panel: {
      label: "Output",
      icon: <Activity />,
      render: (id) => (
        <pre className="whitespace-pre-wrap">
          {id
            ? `$ trestle up ${id}\n${id} is ${state[id] ?? "unknown"}.`
            : "No environment open."}
        </pre>
      ),
    },
  };
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

/** A host's contributions: Doctor and App settings side views, run-state
 *  badges with placement in the tree, a header with Start/Stop, Catalog /
 *  Lineage / Logs sub-tabs (disabled until the environment runs), each
 *  service's state on the canvas, and an Output panel. */
export const WithContributions: Story = {
  render: () => (
    <Shell
      store={createMemoryWorkspaceStore(workspace)}
      contributions={fakeRuntime()}
    />
  ),
};
