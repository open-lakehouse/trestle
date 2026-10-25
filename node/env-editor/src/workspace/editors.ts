// The shell's open editors: each tab is an environment's diagram or one of its
// rendered files. Pure helpers, so the tab rules are testable on their own.

/** An open editor: `env`'s diagram, or its rendered file at `path`. */
export interface EditorTab {
  /** The environment's id. */
  env: string;
  /** The file's path in the environment; absent for its diagram. */
  path?: string;
}

/** A tab's identity. */
export function tabKey(tab: EditorTab): string {
  return `${tab.env}\n${tab.path ?? ""}`;
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
