import { HostProvider } from "@open-lakehouse/host/react";
import { createFakeHost } from "@open-lakehouse/host/testing";
import {
  FIXTURE_CATALOG,
  FIXTURE_PLAN,
} from "@open-lakehouse/stack-wasm/fixtures";
import type { Meta, StoryObj } from "@storybook/react-vite";
import {
  Activity,
  AppWindow,
  Play,
  Settings2,
  ShieldCheck,
  Square,
  Stethoscope,
  Wrench,
} from "lucide-react";
import { type ReactNode, useEffect, useMemo, useState } from "react";
import type { Command } from "../commands/registry.js";
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
import { useWorkbench, WorkbenchProvider } from "./workbench.js";

// The fixture planner: the captured catalog, and one representative plan,
// plus a file listing the selection's modules so Changes has a diff to show.
const planner: Planner = {
  catalog: async () => FIXTURE_CATALOG,
  plan: async (s) => ({
    ...FIXTURE_PLAN,
    files: [
      ...FIXTURE_PLAN.files,
      { path: "modules.txt", contents: `${s.modules.join("\n")}\n` },
    ],
  }),
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
function fakeRuntime(): EditorContributions & {
  /** The host's ⌘K commands for the runtime's current state. */
  commandsNow: () => Command[];
} {
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
  const commandsNow = (): Command[] => [
    ...Object.keys(state).map((id) =>
      running(id)
        ? {
            id: `env.stop.${id}`,
            title: `Stop ${id}`,
            section: "Environment",
            hint: `trestle down ${id}`,
            run: () => transition(id, "stopping", "stopped"),
          }
        : {
            id: `env.start.${id}`,
            title: `Start ${id}`,
            section: "Environment",
            hint: `trestle up ${id}`,
            enabled: state[id] === "stopped",
            reason: `${id} is ${state[id]}`,
            run: () => transition(id, "starting", "running"),
          },
    ),
    {
      id: "env.open-mlflow",
      title: "Open MLflow",
      section: "Environment",
      keywords: ["experiments", "browser"],
      enabled: running("dev"),
      reason: notRunning("dev"),
      run: () => {},
    },
    {
      id: "env.copy-up",
      title: "Copy the `trestle up` command",
      section: "Environment",
      hint: "trestle up dev",
      run: () => void navigator.clipboard?.writeText("trestle up dev"),
    },
    {
      id: "app.check-updates",
      title: "Check for updates",
      section: "App",
      run: () => {},
    },
  ];
  return {
    commandsNow,
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
    activityActions: [
      {
        id: "host-settings",
        label: "Settings",
        icon: <Settings2 />,
        align: "end",
        run: () =>
          window.alert("The host opens its settings, over the workspace."),
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
          details: [
            {
              label: "target",
              value: id === "dev" ? "docker (colima)" : "neon br-staging",
            },
            {
              label: "gateway",
              value: `https://${id}.localhost:8443`,
              copy: true,
            },
          ],
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
      tabs: (id) => [
        ...["Catalog", "Lineage", "Logs"].map((label) => ({
          id: label.toLowerCase(),
          label,
          enabled: running(id),
          reason: notRunning(id),
          render: view(label, `${label} of ${id}, from the host.`),
        })),
        // A host view of the environment's setup, always on.
        {
          id: "policies",
          label: "Policies",
          icon: <ShieldCheck />,
          enabled: true,
          render: view("Policies", `Who may reach what in ${id}.`),
        },
      ],
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

/** A store with drafts only (like browser storage): no history view, and an
 *  environment tab of Diagram and Files. */
export const DraftsOnly: Story = {
  render: () => <Shell store={createMemoryWorkspaceStore(workspace)} />,
};

/** A store that keeps revisions: the History view lists, opens and compares
 *  them, and commits the draft; the Changes tab diffs the draft against the
 *  last revision. */
export const WithRevisions: Story = {
  loaders: [async () => ({ store: await storeWithHistory() })],
  render: (_args, { loaded }) => <Shell store={loaded.store} />,
};

/** A host's contributions: Doctor and App settings side views, run-state
 *  badges with placement in the tree, a header with details (target, a
 *  copyable gateway URL) and Start/Stop, Catalog / Lineage / Logs sub-tabs
 *  (disabled until the environment runs) and an always-on Policies tab, each
 *  service's state on the canvas, and an Output panel. */
export const WithContributions: Story = {
  render: () => <LiveShell />,
};

/** The fake runtime, its commands rebuilt whenever its state changes (a host
 *  would derive them from its query cache the same way). */
function LiveShell() {
  const [runtime] = useState(fakeRuntime);
  const [store] = useState(() => createMemoryWorkspaceStore(workspace));
  const [version, setVersion] = useState(0);
  useEffect(
    () => runtime.environment?.subscribe?.(() => setVersion((v) => v + 1)),
    [runtime],
  );
  // biome-ignore lint/correctness/useExhaustiveDependencies: `version` is the runtime's change signal.
  const contributions = useMemo<EditorContributions>(
    () => ({ ...runtime, commands: runtime.commandsNow() }),
    [runtime, version],
  );
  return <Shell store={store} contributions={contributions} />;
}

/** Host viewers in place of the editor's: a bordered stand-in for Monaco
 *  under Files and in every diff (Changes, comparisons). */
export const WithViewerSeams: Story = {
  loaders: [async () => ({ store: await storeWithHistory() })],
  render: (_args, { loaded }) => (
    <Shell
      store={loaded.store}
      contributions={{
        fileViewer: ({ envId, path, read }) => (
          <pre className="m-3 overflow-auto rounded border-2 border-dashed border-primary p-3 font-mono text-xs">
            {`host file viewer: ${envId}/${path}\n\n${String(read(path))}`}
          </pre>
        ),
        diffViewer: ({ path, original, modified }) => (
          <pre className="m-3 overflow-auto rounded border-2 border-dashed border-primary p-3 font-mono text-xs">
            {`host diff viewer: ${path}\n--- ${original === undefined ? "(new)" : `${original.length} chars`}\n+++ ${modified === undefined ? "(gone)" : `${modified.length} chars`}`}
          </pre>
        ),
      }}
    />
  ),
};

/** A host's workbench view: an Apps item in the activity bar that takes over
 *  the main area (with its own sidebar) while the ⌘K palette, bottom panel
 *  and status bar stay. Each environment's header has an "Open apps" action
 *  that opens it through `useWorkbench()`; the activity-bar item, ⌘K "Show
 *  the editors" or opening an environment switches back, with the diagram
 *  as it was. */
export const WithWorkbenchView: Story = {
  render: () => (
    <WorkbenchProvider>
      <WorkbenchShell />
    </WorkbenchProvider>
  ),
};

function WorkbenchShell() {
  const { open } = useWorkbench();
  const [store] = useState(() => createMemoryWorkspaceStore(workspace));
  const [picked, setPicked] = useState("MLflow");
  const apps = ["MLflow", "Unity Catalog", "Marimo"];
  const row = (label: string): ReactNode => (
    <button
      key={label}
      type="button"
      onClick={() => setPicked(label)}
      className={`block w-full px-3 py-1 text-left text-sm ${label === picked ? "bg-sidebar-accent" : "hover:bg-sidebar-accent/60"}`}
    >
      {label}
    </button>
  );
  const contributions: EditorContributions = {
    workbenchViews: [
      {
        id: "apps",
        label: "Apps",
        icon: <AppWindow />,
        sidebar: () => (
          <div className="py-2">
            <div className="px-3 pb-2 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
              Apps
            </div>
            {apps.map(row)}
          </div>
        ),
        render: () => (
          <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
            {picked}, as the host would embed it.
          </div>
        ),
      },
    ],
    environment: {
      actions: () => [
        {
          id: "open-apps",
          label: "Open apps",
          icon: <AppWindow />,
          run: () => open("apps"),
        },
      ],
    },
  };
  return <Shell store={store} contributions={contributions} />;
}
