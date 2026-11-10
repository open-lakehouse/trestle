// The workspace shell's own commands, for the ⌘K palette and its keymap:
// Go to (environments, open editors, side views, workbench views), View
// (sidebar, panel, problems) and Edit (new environment, commit, diff,
// export). Built from the
// shell's state by a pure function, so the list is testable on its own.

import type { Command } from "../commands/registry.js";
import { type EditorTab, tabKey, tabLabel } from "./editors.js";
import type { EnvDoc, WorkspaceDoc } from "./model.js";

export interface ShellCommandState {
  workspace: WorkspaceDoc;
  /** The open editors, in tab order. */
  open: readonly EditorTab[];
  /** The side views, the editor's and a host's. */
  sideViews: readonly { id: string; label: string; run?: () => void }[];
  activeEnv: EnvDoc | undefined;
  /** A host's workbench views. */
  workbenchViews: readonly { id: string; label: string }[];
  /** The view with the main area: `"editor"` or a workbench view's id. */
  workbench: string;
  /** The store keeps revisions. */
  revisions: boolean;
  /** The planner can bundle (download) environments. */
  canExport: boolean;
  exporting: boolean;
}

export interface ShellCommandActions {
  openEnv: (id: string) => void;
  activate: (tabKey: string) => void;
  /** Give the main area to workbench view `id`, or the editor tabs
   *  (`"editor"`). */
  openWorkbench: (id: string) => void;
  showView: (id: string) => void;
  toggleSidebar: () => void;
  togglePanel: () => void;
  showProblems: () => void;
  newEnv: () => void;
  commit: () => void;
  diff: (envId: string, head: number) => void;
  exportWorkspace: () => void;
}

export function shellCommands(
  state: ShellCommandState,
  actions: ShellCommandActions,
): Command[] {
  const { workspace, open, activeEnv } = state;
  const name = (id: string) =>
    workspace.envs.find((e) => e.id === id)?.name ?? id;
  const commands: Command[] = [
    ...workspace.envs.map((env) => ({
      id: `goto.env.${env.id}`,
      title: `Go to ${env.name}`,
      section: "Go to",
      keywords: ["environment", "diagram", "open"],
      run: () => actions.openEnv(env.id),
    })),
    // Open revisions and comparisons (the environments are above).
    ...open
      .filter((t) => t.revision !== undefined || t.compare)
      .map((tab) => ({
        id: `goto.tab.${tabKey(tab)}`,
        title: `Go to ${tabLabel(tab, name(tab.env))} (${name(tab.env)})`,
        section: "Go to",
        keywords: ["tab", "editor", "revision"],
        run: () => actions.activate(tabKey(tab)),
      })),
    ...state.sideViews.map((view) =>
      view.run
        ? {
            id: `goto.action.${view.id}`,
            title: view.label,
            section: "Go to",
            keywords: ["open"],
            run: view.run,
          }
        : {
            id: `goto.view.${view.id}`,
            title: `Show ${view.label}`,
            section: "Go to",
            keywords: ["view", "sidebar"],
            run: () => actions.showView(view.id),
          },
    ),
    ...workbenchCommands(state, actions),
    // ⌘1…9: the open editors by position.
    ...open.slice(0, 9).map((tab, i) => ({
      id: `goto.tab-index.${i + 1}`,
      title: `Go to editor ${i + 1}`,
      section: "Go to",
      shortcut: `mod+${i + 1}`,
      hidden: true,
      run: () => actions.activate(tabKey(tab)),
    })),
    {
      id: "view.toggle-sidebar",
      title: "Toggle sidebar",
      section: "View",
      shortcut: "mod+b",
      run: actions.toggleSidebar,
    },
    {
      id: "view.toggle-panel",
      title: "Toggle panel",
      section: "View",
      shortcut: "mod+j",
      run: actions.togglePanel,
    },
    {
      id: "view.problems",
      title: "Show problems",
      section: "View",
      keywords: ["errors", "conflicts"],
      run: actions.showProblems,
    },
    {
      id: "edit.new-environment",
      title: "New environment",
      section: "Edit",
      keywords: ["add", "create"],
      run: actions.newEnv,
    },
  ];
  if (state.revisions) {
    commands.push({
      id: "edit.commit",
      title: activeEnv ? `Commit ${activeEnv.name}` : "Commit",
      section: "Edit",
      keywords: ["revision", "save", "history"],
      enabled: !!activeEnv,
      reason: "Open an environment to commit it",
      run: actions.commit,
    });
    const head = activeEnv?.head;
    commands.push({
      id: "edit.diff",
      title: activeEnv
        ? `Diff ${activeEnv.name} against #${head ?? "?"}`
        : "Diff against the last revision",
      section: "Edit",
      keywords: ["compare", "changes", "revision"],
      enabled: !!activeEnv && head !== undefined,
      reason: activeEnv
        ? `${activeEnv.name} has no revision yet`
        : "Open an environment to diff it",
      run: () => {
        if (activeEnv && head !== undefined) actions.diff(activeEnv.id, head);
      },
    });
  }
  if (state.canExport) {
    commands.push({
      id: "edit.export-workspace",
      title: "Export workspace (.zip)",
      section: "Edit",
      keywords: ["download", "bundle", "zip"],
      hint: "For `trestle workspace up`",
      enabled: !state.exporting,
      reason: "Preparing the download…",
      run: actions.exportWorkspace,
    });
  }
  return commands;
}

/** One "Show" command per workbench view, and one back to the editor tabs
 *  while a view has the main area. */
function workbenchCommands(
  { workbenchViews, workbench }: ShellCommandState,
  actions: ShellCommandActions,
): Command[] {
  if (workbenchViews.length === 0) return [];
  const onView = workbenchViews.some((v) => v.id === workbench);
  return [
    ...workbenchViews.map((view) => ({
      id: `goto.workbench.${view.id}`,
      title: `Show ${view.label}`,
      section: "Go to",
      keywords: ["view", "workbench"],
      enabled: view.id !== workbench,
      reason: "Already showing",
      run: () => actions.openWorkbench(view.id),
    })),
    {
      id: "goto.workbench.editor",
      title: "Show the editors",
      section: "Go to",
      keywords: ["editor", "tabs", "environments", "back"],
      enabled: onView,
      reason: "Already showing",
      run: () => actions.openWorkbench("editor"),
    },
  ];
}
