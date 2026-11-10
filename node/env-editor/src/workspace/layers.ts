// An environment tab's layers and their sub-tabs, as pure rules:
// Configure (the editor's Diagram, Files and Changes, then a host's
// configure-layer tabs) and Apps (a host's apps-layer tabs). Each environment
// remembers its layer and the sub-tab picked in each, so switching back lands
// where it was.

import type { ReactNode } from "react";
import type { EnvironmentLayer, EnvironmentTab } from "./contributions.js";

/** The editor's own sub-tabs. */
export const DIAGRAM_TAB = "diagram";
export const FILES_TAB = "files";
export const CHANGES_TAB = "changes";
/** The Diagram tab's id before 0.0.7, still accepted. */
const LEGACY_DESIGN_TAB = "design";

/** One sub-tab, as `EnvSubTabs` lists it. */
export interface SubTab {
  id: string;
  label: string;
  icon?: ReactNode;
  enabled: boolean;
  reason?: string;
}

/** Where an environment tab is: its layer, the sub-tab picked in each, and
 *  the file open under Files. */
export interface EnvView {
  layer: EnvironmentLayer;
  configure: string;
  apps?: string;
  filePath?: string;
}

/** A fresh environment tab: Configure, on its diagram. */
export const INITIAL_VIEW: EnvView = {
  layer: "configure",
  configure: DIAGRAM_TAB,
};

/** What the editor knows about an environment's revisions, for Changes. */
export interface ChangesAvailability {
  /** The store keeps revisions; without, there is no Changes tab. */
  revisions: boolean;
  /** The revision the draft is compared against. */
  head?: number;
}

/** The sub-tabs of each layer. */
export interface LayerTabs {
  configure: SubTab[];
  apps: SubTab[];
}

const subTab = (tab: EnvironmentTab): SubTab => ({
  id: tab.id,
  label: tab.label,
  icon: tab.icon,
  enabled: tab.enabled,
  reason: tab.reason,
});

/** Each layer's sub-tabs: Configure is Diagram, Files, Changes (where the
 *  store keeps revisions) and the host's configure-layer tabs; Apps is the
 *  host's other tabs. */
export function layerTabs(
  hostTabs: readonly EnvironmentTab[],
  changes: ChangesAvailability,
): LayerTabs {
  const configure: SubTab[] = [
    { id: DIAGRAM_TAB, label: "Diagram", enabled: true },
    { id: FILES_TAB, label: "Files", enabled: true },
  ];
  if (changes.revisions) {
    configure.push({
      id: CHANGES_TAB,
      label: "Changes",
      enabled: changes.head !== undefined,
      reason: "No revision yet: commit one to compare the draft against",
    });
  }
  return {
    configure: [
      ...configure,
      ...hostTabs.filter((t) => t.layer === "configure").map(subTab),
    ],
    apps: hostTabs.filter((t) => (t.layer ?? "apps") === "apps").map(subTab),
  };
}

/** Whether a layer has anything to show (Configure always has). */
export function hasLayer(tabs: LayerTabs, layer: EnvironmentLayer): boolean {
  return layer === "configure" || tabs.apps.length > 0;
}

/** The layer and sub-tab to draw for `view`: Apps falls back to Configure
 *  when there are no apps, and a sub-tab that went away (or never existed)
 *  to its layer's first enabled one. A picked tab that is now disabled stays
 *  picked, so it can say why. */
export function resolveView(
  view: EnvView,
  tabs: LayerTabs,
): { layer: EnvironmentLayer; subTab: string } {
  const layer = hasLayer(tabs, view.layer) ? view.layer : "configure";
  const list = tabs[layer];
  const wanted =
    layer === "apps"
      ? view.apps
      : view.configure === LEGACY_DESIGN_TAB
        ? DIAGRAM_TAB
        : view.configure;
  const picked =
    list.find((t) => t.id === wanted) ?? list.find((t) => t.enabled) ?? list[0];
  return { layer, subTab: picked?.id ?? DIAGRAM_TAB };
}

/** `view` on `layer`, keeping each layer's sub-tab. */
export function withLayer(view: EnvView, layer: EnvironmentLayer): EnvView {
  return { ...view, layer };
}

/** `view` with sub-tab `id` picked in `layer`. */
export function withSubTab(
  view: EnvView,
  layer: EnvironmentLayer,
  id: string,
): EnvView {
  return layer === "apps"
    ? { ...view, layer, apps: id }
    : { ...view, layer, configure: id };
}

/** `view` showing the rendered file at `path`: Configure, Files. */
export function withFile(view: EnvView, path: string): EnvView {
  return { ...view, layer: "configure", configure: FILES_TAB, filePath: path };
}
