// The workspace shell's own commands, for the ⌘K palette and its keymap:
// Go to (environments, open editors, side views), View (the active
// environment's layers and sub-tabs, sidebar, panel, problems) and Edit (new
// environment, commit, diff, export). Built from the
// shell's state by a pure function, so the list is testable on its own.

import type { Command } from "../commands/registry.js";
import type { EnvironmentLayer } from "./contributions.js";
import { type EditorTab, tabKey, tabLabel } from "./editors.js";
import { hasLayer, type LayerTabs } from "./layers.js";
import type { EnvDoc, WorkspaceDoc } from "./model.js";

export interface ShellCommandState {
  workspace: WorkspaceDoc;
  /** The open editors, in tab order. */
  open: readonly EditorTab[];
  /** The side views, the editor's and a host's. */
  sideViews: readonly { id: string; label: string; run?: () => void }[];
  activeEnv: EnvDoc | undefined;
  /** The active environment's layers and their sub-tabs. */
  layers?: LayerTabs;
  /** Where the active environment's tab is. */
  current?: { layer: EnvironmentLayer; subTab: string };
  /** The store keeps revisions. */
  revisions: boolean;
  /** The planner can bundle (download) environments. */
  canExport: boolean;
  exporting: boolean;
}

export interface ShellCommandActions {
  openEnv: (id: string) => void;
  activate: (tabKey: string) => void;
  /** Show the active environment's tab on `layer`. */
  showLayer: (layer: EnvironmentLayer) => void;
  /** Show sub-tab `id` of the active environment's `layer`. */
  showSubTab: (layer: EnvironmentLayer, id: string) => void;
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
      keywords: ["environment", "open"],
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
    ...layerCommands(state, actions),
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

/** ⌘1 / ⌘2 switch the active environment between Configure and Apps; each
 *  sub-tab of either has a "Show" command. */
function layerCommands(
  { activeEnv, layers, current }: ShellCommandState,
  actions: ShellCommandActions,
): Command[] {
  if (!activeEnv || !layers) return [];
  const apps = hasLayer(layers, "apps");
  const commands: Command[] = [
    {
      id: "view.layer.configure",
      title: `Configure ${activeEnv.name}`,
      section: "View",
      keywords: ["layer", "diagram", "files", "changes"],
      shortcut: "mod+1",
      run: () => actions.showLayer("configure"),
    },
  ];
  if (apps) {
    commands.push({
      id: "view.layer.apps",
      title: `${activeEnv.name}'s apps`,
      section: "View",
      keywords: ["layer", "apps", "run"],
      shortcut: "mod+2",
      run: () => actions.showLayer("apps"),
    });
  }
  for (const layer of ["configure", "apps"] as const) {
    if (layer === "apps" && !apps) continue;
    for (const tab of layers[layer]) {
      commands.push({
        id: `view.${layer}.${tab.id}`,
        title: `Show ${tab.label}`,
        section: "View",
        keywords: [layer, activeEnv.name],
        hint: layer === "apps" ? "Apps" : "Configure",
        enabled:
          tab.enabled &&
          !(current?.layer === layer && current.subTab === tab.id),
        reason: tab.enabled ? "Already showing" : tab.reason,
        run: () => actions.showSubTab(layer, tab.id),
      });
    }
  }
  return commands;
}
