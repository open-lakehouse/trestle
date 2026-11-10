import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import { CommandList } from "../commands/CommandPalette.js";
import { filterCommands } from "../commands/registry.js";
import { sideViewItems } from "./ActivityBar.js";
import { layerTabs } from "./layers.js";
import type { WorkspaceDoc } from "./model.js";
import { type ShellCommandActions, shellCommands } from "./shellCommands.js";

const workspace: WorkspaceDoc = {
  name: "local",
  mesh: false,
  envs: [
    { id: "dev", name: "dev", slot: 0, head: 2 },
    { id: "prod", name: "prod", slot: 1 },
  ],
};

function actions(log: string[]): ShellCommandActions {
  const note =
    (name: string) =>
    (...args: unknown[]) =>
      log.push([name, ...args].join(":"));
  return {
    openEnv: note("openEnv"),
    activate: note("activate"),
    showLayer: note("showLayer"),
    showSubTab: note("showSubTab"),
    showView: note("showView"),
    toggleSidebar: note("toggleSidebar"),
    togglePanel: note("togglePanel"),
    showProblems: note("showProblems"),
    newEnv: note("newEnv"),
    commit: note("commit"),
    diff: note("diff"),
    exportWorkspace: note("exportWorkspace"),
  };
}

const build = (
  log: string[],
  overrides: Partial<Parameters<typeof shellCommands>[0]> = {},
) =>
  shellCommands(
    {
      workspace,
      open: [{ env: "dev" }, { env: "dev", revision: 1 }],
      sideViews: sideViewItems(false),
      activeEnv: workspace.envs[0],
      revisions: false,
      canExport: false,
      exporting: false,
      ...overrides,
    },
    actions(log),
  );

describe("the shell's commands", () => {
  test("without a host: Go to, Edit and View only", () => {
    const sections = filterCommands(build([]), "").map(([s]) => s);
    expect(sections).toEqual(["Go to", "Edit", "View"]);
    const html = renderToStaticMarkup(
      <CommandList
        groups={filterCommands(build([]), "")}
        activeKey={undefined}
        onPick={() => {}}
        onHover={() => {}}
        query=""
      />,
    );
    expect(html).toContain("Go to dev");
    expect(html).toContain("Go to #1 (dev)");
    expect(html).toContain("Show Workspace settings");
    expect(html).not.toContain("Show Modules");
    expect(html).toContain("Toggle sidebar");
    expect(html).not.toContain("Go to editor 1");
  });

  test("⌘B and ⌘J are commands; no positional editor jumps", () => {
    const log: string[] = [];
    const byShortcut = (s: string) =>
      build(log).find((c) => [c.shortcut].flat().includes(s));
    byShortcut("mod+b")?.run();
    byShortcut("mod+j")?.run();
    expect(log).toEqual(["toggleSidebar", "togglePanel"]);
    for (const n of [1, 2, 3, 9]) {
      expect(byShortcut(`mod+${n}`)).toBeUndefined();
    }
  });

  const appsTab = {
    id: "catalog",
    label: "Catalog",
    enabled: true,
    render: () => null,
  };

  test("⌘1 shows Configure; ⌘2 Apps, only when there are apps", () => {
    const log: string[] = [];
    const configureOnly = build(log, {
      layers: layerTabs([], { revisions: true, head: 2 }),
      current: { layer: "configure", subTab: "diagram" },
    });
    const byShortcut = (list: typeof configureOnly, s: string) =>
      list.find((c) => [c.shortcut].flat().includes(s));
    byShortcut(configureOnly, "mod+1")?.run();
    expect(byShortcut(configureOnly, "mod+2")).toBeUndefined();
    const withApps = build(log, {
      layers: layerTabs([appsTab], { revisions: false }),
      current: { layer: "configure", subTab: "diagram" },
    });
    byShortcut(withApps, "mod+2")?.run();
    expect(log).toEqual(["showLayer:configure", "showLayer:apps"]);
  });

  test("each sub-tab has a Show command; the current one is disabled", () => {
    const log: string[] = [];
    const list = build(log, {
      layers: layerTabs(
        [
          appsTab,
          {
            id: "logs",
            label: "Logs",
            enabled: false,
            reason: "dev isn't running",
            render: () => null,
          },
        ],
        { revisions: true },
      ),
      current: { layer: "configure", subTab: "diagram" },
    });
    const show = (id: string) => list.find((c) => c.id === id);
    expect(show("view.configure.diagram")?.enabled).toBe(false);
    expect(show("view.configure.changes")?.enabled).toBe(false);
    expect(show("view.apps.logs")?.reason).toBe("dev isn't running");
    show("view.configure.files")?.run();
    show("view.apps.catalog")?.run();
    expect(log).toEqual([
      "showSubTab:configure:files",
      "showSubTab:apps:catalog",
    ]);
  });

  test("commit and diff come with revisions; diff needs a head", () => {
    const log: string[] = [];
    const withRevisions = build(log, { revisions: true });
    withRevisions.find((c) => c.id === "edit.diff")?.run();
    withRevisions.find((c) => c.id === "edit.commit")?.run();
    expect(log).toEqual(["diff:dev:2", "commit"]);
    const prod = build(log, { revisions: true, activeEnv: workspace.envs[1] });
    expect(prod.find((c) => c.id === "edit.diff")?.enabled).toBe(false);
    expect(build(log).some((c) => c.id === "edit.commit")).toBe(false);
  });

  test("export only where the planner bundles", () => {
    expect(build([]).some((c) => c.id === "edit.export-workspace")).toBe(false);
    expect(
      build([], { canExport: true }).some(
        (c) => c.id === "edit.export-workspace",
      ),
    ).toBe(true);
  });
});
