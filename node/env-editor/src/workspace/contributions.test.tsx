import { describe, expect, test } from "bun:test";
import { TooltipProvider } from "@open-lakehouse/ui-kit";
import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { ActivityBar, sideViewItems } from "./ActivityBar.js";
import {
  activePanelTab,
  BottomPanelProvider,
  createBottomPanel,
  useBottomPanel,
} from "./bottomPanel.js";
import {
  ContributionsProvider,
  type EditorContributions,
  hostPanels,
  type PanelContribution,
  useContributions,
} from "./contributions.js";
import { EnvTree } from "./EnvTree.js";
import { type EnvView, INITIAL_VIEW } from "./editors.js";
import {
  designTab,
  EnvHeader,
  EnvironmentPane,
  EnvSubTabs,
} from "./envHeader.js";
import type { WorkspaceDoc } from "./model.js";
import { shellCommands } from "./shellCommands.js";
import { createMemoryWorkspaceStore } from "./store.js";
import { WorkspaceProvider } from "./useWorkspace.js";
import { useDiffViewer, useFileViewer } from "./viewers.js";
import { BottomPanel } from "./WorkspaceShell.js";
import {
  createWorkbench,
  useWorkbench,
  WorkbenchArea,
  WorkbenchProvider,
} from "./workbench.js";

const html = (node: ReactNode) =>
  renderToStaticMarkup(<TooltipProvider>{node}</TooltipProvider>);

describe("side views", () => {
  test("a host's views follow the editor's, end-aligned ones last", () => {
    const ids = sideViewItems(false, [
      { id: "settings-app", label: "App settings", icon: null, align: "end" },
      { id: "doctor", label: "Doctor", icon: null, render: () => null },
    ] as never).map((i) => i.id);
    expect(ids).toEqual(["explorer", "settings", "settings-app", "doctor"]);
    const out = html(
      <ActivityBar
        view="doctor"
        onView={() => {}}
        problemCount={0}
        history
        extra={[
          { id: "doctor", label: "Doctor", icon: null, render: () => null },
          {
            id: "app",
            label: "App settings",
            icon: null,
            align: "end",
            render: () => null,
          },
        ]}
      />,
    );
    expect(out).toContain('aria-label="History"');
    expect(out).toContain('aria-label="Doctor" aria-pressed="true"');
    // The end-aligned view sits in the bottom group, after the others.
    expect(out.indexOf("mt-auto")).toBeLessThan(
      out.indexOf('aria-label="App settings"'),
    );
    expect(out.indexOf('aria-label="Doctor"')).toBeLessThan(
      out.indexOf("mt-auto"),
    );
  });

  test("without contributions only the editor's views are listed", () => {
    expect(sideViewItems(true).map((i) => i.id)).toEqual([
      "explorer",
      "history",
      "settings",
    ]);
  });

  test("a host's actions come last, and run instead of opening a view", () => {
    let ran = 0;
    const items = sideViewItems(
      false,
      [{ id: "doctor", label: "Doctor", icon: null, render: () => null }],
      [
        {
          id: "gear",
          label: "Settings",
          icon: null,
          align: "end",
          run: () => ran++,
        },
      ],
    );
    expect(items.map((i) => i.id)).toEqual([
      "explorer",
      "settings",
      "doctor",
      "gear",
    ]);
    items.at(-1)?.run?.();
    expect(ran).toBe(1);
    const out = html(
      <ActivityBar
        view="explorer"
        onView={() => {}}
        problemCount={0}
        history={false}
        actions={[
          {
            id: "gear",
            label: "Settings",
            icon: null,
            align: "end",
            run: () => {},
          },
        ]}
      />,
    );
    // An action isn't a view: it is never shown as the open one.
    expect(out).toContain('aria-label="Settings"');
    expect(out).not.toContain('aria-label="Settings" aria-pressed');
    expect(out.indexOf("mt-auto")).toBeLessThan(
      out.indexOf('aria-label="Settings"'),
    );
  });
});

describe("environment header", () => {
  test("shows the state, revision, placement and actions", () => {
    let started = 0;
    const out = html(
      <EnvHeader
        status={{ state: "running", revision: 3, placement: "local slot 0" }}
        actions={[
          { id: "stop", label: "Stop", run: () => started++ },
          {
            id: "logs",
            label: "Open logs",
            enabled: false,
            reason: "No logs yet",
            run: () => {},
          },
        ]}
      />,
    );
    expect(out).toContain('data-run-state="running"');
    expect(out).toContain(">running<");
    expect(out).toContain("rev 3");
    expect(out).toContain("local slot 0");
    expect(out).toContain("Stop");
    // The disabled action says why.
    expect(out).toMatch(/title="No logs yet"[^>]*>\s*<button[^>]*disabled/);
  });

  test("names the environment and shows details, copyable where asked", () => {
    const out = html(
      <EnvHeader
        name="dev"
        status={{
          state: "running",
          details: [
            { label: "target", value: "branch preview" },
            { label: "gateway", value: "https://dev.localhost", copy: true },
          ],
        }}
        details={[{ label: "editor", value: "replaced by the host's" }]}
        actions={[]}
      />,
    );
    expect(out).toContain(">dev<");
    expect(out).toContain("branch preview");
    expect(out).toContain('aria-label="Copy gateway"');
    expect(out).not.toContain('aria-label="Copy target"');
    expect(out).not.toContain("replaced by the host");
  });
});

describe("environment sub-tabs", () => {
  test("Diagram first; a disabled tab says why and can't be picked", () => {
    const picked: string[] = [];
    const out = html(
      <EnvSubTabs
        tabs={[
          designTab,
          { id: "catalog", label: "Catalog", enabled: true },
          {
            id: "logs",
            label: "Logs",
            enabled: false,
            reason: "dev isn't running",
          },
        ]}
        active="catalog"
        onSelect={(id) => picked.push(id)}
      />,
    );
    expect(out.indexOf("Diagram")).toBeLessThan(out.indexOf("Catalog"));
    expect(out).toMatch(/aria-selected="true"[^>]*>(<svg.*?<\/svg>)?Catalog/);
    expect(out).toContain(
      'aria-selected="false" aria-disabled="true" title="dev isn&#x27;t running"',
    );
  });
});

const workspace: WorkspaceDoc = {
  name: "local",
  mesh: false,
  envs: [
    { id: "dev", name: "dev", slot: 0 },
    { id: "prod", name: "prod", slot: 1 },
  ],
};

function Tree({
  contributions,
  plans = {},
}: {
  contributions?: EditorContributions;
  plans?: Record<string, never>;
}) {
  return (
    <WorkspaceProvider store={createMemoryWorkspaceStore(workspace)}>
      <ContributionsProvider contributions={contributions}>
        <EnvTree
          workspace={workspace}
          plans={plans}
          conflicts={[]}
          active={undefined}
          onOpen={() => {}}
          downloading={false}
        />
      </ContributionsProvider>
    </WorkspaceProvider>
  );
}

describe("environment tree", () => {
  test("shows a host's state and placement per environment", () => {
    const out = html(
      <Tree
        contributions={{
          environment: {
            status: (id) =>
              id === "dev"
                ? { state: "running", placement: "local slot 0" }
                : { state: "stopped", placement: "branch preview" },
          },
        }}
      />,
    );
    expect(out).toContain('data-run-state="running"');
    expect(out).toContain('data-run-state="stopped"');
    expect(out).toContain("local slot 0");
    expect(out).toContain("branch preview");
  });

  test("without contributions it is unchanged: no badges, slot offsets", () => {
    const out = html(<Tree />);
    expect(out).not.toContain("data-run-state");
    expect(out).toContain("+0");
    expect(out).toContain("+1000");
  });

  test("one row per environment: no diagram or file rows", () => {
    const plan = {
      plan: {
        files: [{ path: "modules/postgres/compose.yaml", contents: "" }],
        gateway: { listeners: [] },
      },
    };
    const out = html(<Tree plans={{ dev: plan, prod: plan } as never} />);
    expect(out.match(/role="treeitem"/g)?.length).toBe(2);
    expect(out).not.toContain("Diagram");
    expect(out).not.toContain("compose.yaml");
  });
});

describe("ContributionsProvider", () => {
  test("hands the contributions down, and none by default", () => {
    function Probe() {
      const { contributions, version } = useContributions();
      return (
        <span>
          {contributions?.panel?.label ?? "none"}:{version}
        </span>
      );
    }
    expect(renderToStaticMarkup(<Probe />)).toContain("none:0");
    expect(
      renderToStaticMarkup(
        <ContributionsProvider
          contributions={{ panel: { label: "Output", render: () => null } }}
        >
          <Probe />
        </ContributionsProvider>,
      ),
    ).toContain("Output");
  });
});

describe("EnvironmentPane", () => {
  const pane = (
    contributions?: EditorContributions,
    view: Partial<EnvView> = {},
    revisions: { revisions: boolean; head?: number } = { revisions: false },
  ) =>
    html(
      <ContributionsProvider contributions={contributions}>
        <EnvironmentPane
          envId="dev"
          name="dev"
          view={{ ...INITIAL_VIEW, ...view }}
          onView={() => {}}
          revisions={revisions}
          details={[
            { label: "gateway", value: "https://dev.localhost", copy: true },
          ]}
          diagram={<div id="diagram" />}
          files={<div id="files" />}
          changes={<div id="changes" />}
        />
      </ContributionsProvider>,
    );

  test("without contributions: the header, the editor's tabs, the diagram", () => {
    const out = pane();
    expect(out).toContain(">dev<");
    expect(out).toContain("https://dev.localhost");
    expect(out).toContain('aria-label="Copy gateway"');
    expect(out).toContain('aria-label="Environment views"');
    expect(out).toContain(">Diagram<");
    expect(out).toContain(">Files<");
    expect(out).not.toContain(">Changes<");
    expect(out).toContain('<div id="diagram"></div>');
  });

  test("Files and Changes render their bodies; Changes needs a revision", () => {
    expect(pane(undefined, { subTab: "files" })).toContain(
      '<div id="files"></div>',
    );
    const unrevised = pane(
      undefined,
      { subTab: "changes" },
      { revisions: true },
    );
    expect(unrevised).toContain("Changes is unavailable: No revision yet");
    expect(unrevised).not.toContain('id="changes"');
    expect(
      pane(undefined, { subTab: "changes" }, { revisions: true, head: 1 }),
    ).toContain('<div id="changes"></div>');
  });

  const host: EditorContributions = {
    environment: {
      status: () => ({ state: "stopped" }),
      tabs: (id) => [
        {
          id: "catalog",
          label: "Catalog",
          enabled: false,
          reason: `${id} isn't running`,
          render: () => <div id="catalog" />,
        },
        {
          id: "logs",
          label: "Logs",
          enabled: true,
          render: () => <div id="logs" />,
        },
      ],
    },
  };

  test("a host's status and tabs follow the editor's", () => {
    const out = pane(host);
    expect(out).toContain('data-run-state="stopped"');
    expect(out.indexOf(">Files<")).toBeLessThan(out.indexOf(">Catalog<"));
    expect(out).toContain('<div id="diagram"></div>');
  });

  test("an enabled host tab renders in place of the editor", () => {
    const out = pane(host, { subTab: "logs" });
    expect(out).toContain('<div id="logs"></div>');
    expect(out).not.toContain('id="diagram"');
  });

  test("a disabled host tab says why instead of rendering", () => {
    const out = pane(host, { subTab: "catalog" });
    expect(out).toContain("Catalog is unavailable: dev isn&#x27;t running");
    expect(out).not.toContain('id="catalog"');
  });
});

describe("workbench views", () => {
  const apps = {
    id: "apps",
    label: "Apps",
    icon: null,
    sidebar: () => <nav id="apps-sidebar" />,
    render: () => <div id="apps" />,
  };
  const area = (active: string) =>
    html(
      <WorkbenchArea views={[apps]} active={active}>
        <div id="editor" />
      </WorkbenchArea>,
    );
  const editorHidden = /data-workbench-editor=""[^>]*hidden/;

  test("the editor tabs have the main area by default", () => {
    const out = area("editor");
    expect(out).toContain('<div id="editor"></div>');
    expect(out).not.toContain('id="apps"');
    expect(out).not.toMatch(editorHidden);
  });

  test("an active view takes over; the editor stays mounted, hidden", () => {
    const out = area("apps");
    expect(out).toContain('<div id="apps"></div>');
    expect(out).toContain('<div id="editor"></div>');
    expect(out).toMatch(editorHidden);
  });

  test("an unknown view is the editor", () => {
    expect(area("gone")).not.toMatch(editorHidden);
  });

  test("its activity-bar item comes after the host's side views", () => {
    const ids = sideViewItems(
      false,
      [{ id: "doctor", label: "Doctor", icon: null, render: () => null }],
      [{ id: "gear", label: "Settings", icon: null, run: () => {} }],
      [apps],
    ).map((i) => [i.id, !!i.workbench]);
    expect(ids).toEqual([
      ["explorer", false],
      ["settings", false],
      ["doctor", false],
      ["apps", true],
      ["gear", false],
    ]);
    const out = html(
      <ActivityBar
        view="apps"
        onView={() => {}}
        problemCount={0}
        history={false}
        workbench={[apps]}
      />,
    );
    expect(out).toContain('aria-label="Apps" aria-pressed="true"');
  });

  test("open() switches, from a host's code and from a command", () => {
    const store = createWorkbench();
    function Probe() {
      const { active } = useWorkbench();
      return (
        <WorkbenchArea views={[apps]} active={active}>
          <div id="editor" />
        </WorkbenchArea>
      );
    }
    const render = () =>
      html(
        <WorkbenchProvider workbench={store}>
          <Probe />
        </WorkbenchProvider>,
      );
    expect(render()).not.toContain('id="apps"');
    const seen: string[] = [];
    const unsubscribe = store.subscribe(() => seen.push(store.active()));
    // A host action (say, in an environment's header) opens the view.
    store.open("apps");
    expect(render()).toContain('<div id="apps"></div>');
    // The shell's ⌘K command back to the editors.
    const back = shellCommands(
      {
        workspace,
        open: [],
        sideViews: [],
        activeEnv: undefined,
        workbenchViews: [apps],
        workbench: store.active(),
        revisions: false,
        canExport: false,
        exporting: false,
      },
      {
        openWorkbench: store.open,
      } as unknown as Parameters<typeof shellCommands>[1],
    ).find((c) => c.id === "goto.workbench.editor");
    expect(back?.enabled).toBe(true);
    back?.run();
    expect(render()).not.toContain('id="apps"');
    expect(seen).toEqual(["apps", "editor"]);
    unsubscribe();
  });

  test("useWorkbench needs a provider", () => {
    function Bare() {
      useWorkbench();
      return null;
    }
    expect(() => html(<Bare />)).toThrow("WorkbenchProvider");
  });
});

describe("viewers", () => {
  function Viewers() {
    const file = useFileViewer();
    const diff = useDiffViewer();
    return (
      <>
        {file({ envId: "dev", path: "a.yaml", read: () => "x: 1" })}
        {diff({
          envId: "dev",
          path: "a.yaml",
          original: "x: 1",
          modified: "x: 2",
        })}
      </>
    );
  }

  test("the editor's own by default", () => {
    const out = html(<Viewers />);
    expect(out).toContain("a.yaml");
    expect(out).toContain('aria-label="Changes to a.yaml"');
    expect(out).toContain("x: 2");
  });

  test("a host's in their place", () => {
    const out = html(
      <ContributionsProvider
        contributions={{
          fileViewer: ({ envId, path }) => <i id={`file-${envId}-${path}`} />,
          diffViewer: ({ path, modified }) => (
            <i id={`diff-${path}`}>{modified}</i>
          ),
        }}
      >
        <Viewers />
      </ContributionsProvider>,
    );
    expect(out).toContain('<i id="file-dev-a.yaml"></i>');
    expect(out).toContain('<i id="diff-a.yaml">x: 2</i>');
    expect(out).not.toContain("Changes to");
  });
});

describe("bottom panel", () => {
  const panel = (id: string, label: string): PanelContribution => ({
    id,
    label,
    render: (envId) => <div id={`${id}-body`}>{`${label} for ${envId}`}</div>,
  });
  const run = panel("run", "Run");
  const logs = panel("logs", "Logs");
  const bottom = (
    view: string,
    panels: PanelContribution[] = [],
    env: string | undefined = undefined,
  ) =>
    html(
      <BottomPanel
        height={180}
        view={view}
        onView={() => {}}
        onClose={() => {}}
        workspace={workspace}
        env={env ? { id: env, name: env, slot: 0 } : undefined}
        plans={{}}
        conflicts={[]}
        notice={null}
        onOpen={() => {}}
        panels={panels}
      />,
    );
  // The tab labels, in order, and which one is selected.
  const tabs = (out: string) =>
    [
      ...out.matchAll(
        /role="tab" aria-selected="(true|false)"[^>]*>(?:<svg.*?<\/svg>)?([^<]*)/g,
      ),
    ].map((m) => [m[2], m[1] === "true"] as const);

  test("two host panels are two tabs after Problems, each with its content", () => {
    const panels = [run, logs];
    expect(tabs(bottom("problems", panels))).toEqual([
      ["Problems", true],
      ["Run", false],
      ["Logs", false],
    ]);
    // Each selected id shows that panel's content, and only that one.
    const onRun = bottom("run", panels, "dev");
    expect(onRun).toContain('<div id="run-body">Run for dev</div>');
    expect(onRun).not.toContain("logs-body");
    const onLogs = bottom("logs", panels, "dev");
    expect(onLogs).toContain('<div id="logs-body">Logs for dev</div>');
    expect(onLogs).not.toContain("run-body");
    expect(tabs(onLogs)).toEqual([
      ["Problems", false],
      ["Run", false],
      ["Logs", true],
    ]);
  });

  test("without host panels the static Run output is the one tab", () => {
    expect(tabs(bottom("output"))).toEqual([
      ["Problems", false],
      ["Run", true],
    ]);
    expect(bottom("output")).toContain("No environment open.");
  });

  test("a selected panel that vanished falls back to the first host panel", () => {
    expect(tabs(bottom("gone", [run, logs]))).toEqual([
      ["Problems", false],
      ["Run", true],
      ["Logs", false],
    ]);
    expect(bottom("gone", [run, logs])).toContain("run-body");
    // The static output is gone too once the host has panels.
    expect(tabs(bottom("output", [logs]))[1]).toEqual(["Logs", true]);
  });

  test("with no panels at all, a vanished selection falls back to Problems", () => {
    expect(tabs(bottom("logs"))).toEqual([
      ["Problems", true],
      ["Run", false],
    ]);
    expect(activePanelTab("logs", [])).toBe("problems");
  });

  test("the pure fallback rules", () => {
    expect(activePanelTab("logs", [run, logs])).toBe("logs");
    expect(activePanelTab("problems", [run, logs])).toBe("problems");
    expect(activePanelTab("gone", [run, logs])).toBe("run");
    expect(activePanelTab("output", [])).toBe("output");
    expect(activePanelTab("output", [run])).toBe("run");
  });

  test('the legacy `panel` is one panel with the id "output"', () => {
    const legacy = { label: "Output", render: () => <i id="legacy" /> };
    const contributions: EditorContributions = { panel: legacy };
    const [only] = hostPanels(contributions);
    expect(only?.id).toBe("output");
    expect(hostPanels(contributions)).toHaveLength(1);
    const out = bottom("output", hostPanels(contributions));
    expect(tabs(out)).toEqual([
      ["Problems", false],
      ["Output", true],
    ]);
    expect(out).toContain('<i id="legacy"></i>');
    // And it keeps its place when a host also has panels.
    expect(
      hostPanels({ panels: [logs], panel: legacy }).map((p) => p.id),
    ).toEqual(["logs", "output"]);
  });

  test("hostPanels drops the reserved id and repeated ids", () => {
    const ids = hostPanels({
      panels: [panel("problems", "Mine"), logs, panel("logs", "Again")],
      panel: { label: "Output", render: () => null },
    }).map((p) => [p.id, p.label]);
    expect(ids).toEqual([
      ["logs", "Logs"],
      ["output", "Output"],
    ]);
    expect(hostPanels(undefined)).toEqual([]);
  });

  test("useBottomPanel().open shows a tab, from outside the shell", () => {
    const store = createBottomPanel({ shown: false });
    let open: ((id: string) => void) | undefined;
    function Host() {
      open = useBottomPanel().open;
      return null;
    }
    renderToStaticMarkup(
      <BottomPanelProvider panel={store}>
        <Host />
      </BottomPanelProvider>,
    );
    const seen: string[] = [];
    const unsubscribe = store.subscribe(() => seen.push(store.state().view));
    // A host's ⌘K command, say: shows the panel on its Logs tab.
    open?.("logs");
    expect(store.state()).toEqual({ view: "logs", shown: true });
    expect(seen).toEqual(["logs"]);
    unsubscribe();
  });

  test("useBottomPanel needs a provider; the shell's own store starts on Problems, shown", () => {
    function Bare() {
      useBottomPanel();
      return null;
    }
    expect(() => html(<Bare />)).toThrow("BottomPanelProvider");
    expect(createBottomPanel().state()).toEqual({
      view: "problems",
      shown: true,
    });
  });
});
