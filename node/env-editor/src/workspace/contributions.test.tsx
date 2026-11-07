import { describe, expect, test } from "bun:test";
import { TooltipProvider } from "@open-lakehouse/ui-kit";
import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { ActivityBar, sideViewItems } from "./ActivityBar.js";
import {
  ContributionsProvider,
  type EditorContributions,
  useContributions,
} from "./contributions.js";
import { EnvTree } from "./EnvTree.js";
import {
  designTab,
  EnvHeader,
  EnvironmentPane,
  EnvSubTabs,
} from "./envHeader.js";
import type { WorkspaceDoc } from "./model.js";
import { createMemoryWorkspaceStore } from "./store.js";
import { WorkspaceProvider } from "./useWorkspace.js";

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
});

describe("environment sub-tabs", () => {
  test("Design first; a disabled tab says why and can't be picked", () => {
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
    expect(out.indexOf("Design")).toBeLessThan(out.indexOf("Catalog"));
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

function Tree({ contributions }: { contributions?: EditorContributions }) {
  return (
    <WorkspaceProvider store={createMemoryWorkspaceStore(workspace)}>
      <ContributionsProvider contributions={contributions}>
        <EnvTree
          workspace={workspace}
          plans={{}}
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
  const pane = (contributions?: EditorContributions, subTab = "design") =>
    html(
      <ContributionsProvider contributions={contributions}>
        <EnvironmentPane envId="dev" subTab={subTab} onSubTab={() => {}}>
          <div id="design" />
        </EnvironmentPane>
      </ContributionsProvider>,
    );

  test("without contributions it is the editor alone", () => {
    expect(pane()).toBe('<div id="design"></div>');
    expect(pane({ environment: { status: () => undefined } })).toBe(
      '<div id="design"></div>',
    );
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

  test("a host's header and sub-tabs frame the editor", () => {
    const out = pane(host);
    expect(out).toContain('data-run-state="stopped"');
    expect(out).toContain('aria-label="Environment views"');
    expect(out).toContain('<div id="design"></div>');
  });

  test("an enabled host tab renders in place of the editor", () => {
    const out = pane(host, "logs");
    expect(out).toContain('<div id="logs"></div>');
    expect(out).not.toContain('id="design"');
  });

  test("a disabled host tab says why instead of rendering", () => {
    const out = pane(host, "catalog");
    expect(out).toContain("Catalog is unavailable: dev isn&#x27;t running");
    expect(out).not.toContain('id="catalog"');
  });
});
