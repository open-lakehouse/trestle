import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import { CommandList } from "../commands/CommandPalette.js";
import { filterCommands } from "../commands/registry.js";
import { sideViewItems } from "./ActivityBar.js";
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
    openWorkbench: note("openWorkbench"),
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
      workbenchViews: [],
      workbench: "editor",
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

  test("⌘B, ⌘J and ⌘1…9 are commands", () => {
    const log: string[] = [];
    const byShortcut = (s: string) =>
      build(log).find((c) => [c.shortcut].flat().includes(s));
    byShortcut("mod+b")?.run();
    byShortcut("mod+j")?.run();
    byShortcut("mod+2")?.run();
    expect(byShortcut("mod+3")).toBeUndefined();
    expect(log).toEqual([
      "toggleSidebar",
      "togglePanel",
      "activate:dev\nrevision:1",
    ]);
  });

  test("a Show command per workbench view, and one back to the editors", () => {
    const log: string[] = [];
    const views = [{ id: "apps", label: "Apps" }];
    const onEditor = build(log, { workbenchViews: views });
    const find = (list: typeof onEditor, id: string) =>
      list.find((c) => c.id === id);
    expect(find(onEditor, "goto.workbench.editor")?.enabled).toBe(false);
    find(onEditor, "goto.workbench.apps")?.run();
    const onApps = build(log, { workbenchViews: views, workbench: "apps" });
    expect(find(onApps, "goto.workbench.apps")?.enabled).toBe(false);
    find(onApps, "goto.workbench.editor")?.run();
    expect(log).toEqual(["openWorkbench:apps", "openWorkbench:editor"]);
    expect(build(log).some((c) => c.id.startsWith("goto.workbench"))).toBe(
      false,
    );
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
