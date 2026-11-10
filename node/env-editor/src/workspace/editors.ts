// The shell's open editors: each tab is an environment (its diagram, files,
// changes and a host's sub-tabs), one of its revisions or a comparison of
// two. Pure helpers, so the tab rules are testable on their own.

import type { ReactNode } from "react";

/** An open editor: environment `env`, one of its revisions (read-only), or a
 *  comparison of two of them. At most one of `revision` and `compare` is
 *  set. */
export interface EditorTab {
  /** The environment's id. */
  env: string;
  /** A revision's number, to view it read-only. */
  revision?: number;
  /** Two revisions to compare; `"draft"` is the environment's draft. */
  compare?: { from: number; to: number | "draft" };
}

/** A tab's identity. */
export function tabKey(tab: EditorTab): string {
  const what = tab.compare
    ? `compare:${tab.compare.from}..${tab.compare.to}`
    : tab.revision !== undefined
      ? `revision:${tab.revision}`
      : "";
  return `${tab.env}\n${what}`;
}

/** `tabs` with `tab` open (appended unless it already is). */
export function openTab(
  tabs: readonly EditorTab[],
  tab: EditorTab,
): EditorTab[] {
  const key = tabKey(tab);
  return tabs.some((t) => tabKey(t) === key) ? [...tabs] : [...tabs, tab];
}

/** `tabs` without the one keyed `key`. */
export function closeTab(tabs: readonly EditorTab[], key: string): EditorTab[] {
  return tabs.filter((t) => tabKey(t) !== key);
}

/** `tabs` without those of environments not in `envIds`; the same array when
 *  nothing closed. */
export function pruneTabs(
  tabs: EditorTab[],
  envIds: ReadonlySet<string>,
): EditorTab[] {
  const kept = tabs.filter((t) => envIds.has(t.env));
  return kept.length === tabs.length ? tabs : kept;
}

/** The tab to activate once the active one closes: its right neighbour, else
 *  its left one. */
export function neighbourAfterClose(
  tabs: readonly EditorTab[],
  key: string,
): string | undefined {
  const i = tabs.findIndex((t) => tabKey(t) === key);
  const next = tabs[i + 1] ?? tabs[i - 1];
  return next && tabKey(next);
}

/** A tab's label: `#n` for a revision, `#a → b` for a comparison, else the
 *  environment's name. */
export function tabLabel(tab: EditorTab, envName: string): string {
  if (tab.compare) {
    const to = tab.compare.to === "draft" ? "draft" : `#${tab.compare.to}`;
    return `#${tab.compare.from} → ${to}`;
  }
  if (tab.revision !== undefined) return `#${tab.revision}`;
  return envName;
}

// ---------------------------------------------------------------------------
// An environment tab's sub-tabs: the editor's Diagram, Files and Changes, then
// a host's. Each environment remembers the one picked and the file open under
// Files.
// ---------------------------------------------------------------------------

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

/** Where an environment tab is: its sub-tab, and the file open under Files. */
export interface EnvView {
  subTab: string;
  filePath?: string;
}

/** A fresh environment tab, on its diagram. */
export const INITIAL_VIEW: EnvView = { subTab: DIAGRAM_TAB };

/** What the editor knows about an environment's revisions, for Changes. */
export interface ChangesAvailability {
  /** The store keeps revisions; without, there is no Changes tab. */
  revisions: boolean;
  /** The revision the draft is compared against. */
  head?: number;
}

/** An environment tab's sub-tabs: Diagram, Files, Changes (where the store
 *  keeps revisions; enabled once there is one), then the host's. */
export function envSubTabs(
  hostTabs: readonly SubTab[],
  changes: ChangesAvailability,
): SubTab[] {
  const tabs: SubTab[] = [
    { id: DIAGRAM_TAB, label: "Diagram", enabled: true },
    { id: FILES_TAB, label: "Files", enabled: true },
  ];
  if (changes.revisions) {
    tabs.push({
      id: CHANGES_TAB,
      label: "Changes",
      enabled: changes.head !== undefined,
      reason: "No revision yet: commit one to compare the draft against",
    });
  }
  return [
    ...tabs,
    ...hostTabs.map(({ id, label, icon, enabled, reason }) => ({
      id,
      label,
      icon,
      enabled,
      reason,
    })),
  ];
}

/** The sub-tab to draw for `view`: the picked one while it exists (even
 *  disabled, so it can say why), else the first enabled one. */
export function resolveSubTab(view: EnvView, tabs: readonly SubTab[]): string {
  const wanted = view.subTab === LEGACY_DESIGN_TAB ? DIAGRAM_TAB : view.subTab;
  return (
    tabs.find((t) => t.id === wanted)?.id ??
    tabs.find((t) => t.enabled)?.id ??
    DIAGRAM_TAB
  );
}

/** `view` showing the rendered file at `path`, under Files. */
export function withFile(view: EnvView, path: string): EnvView {
  return { ...view, subTab: FILES_TAB, filePath: path };
}
