// The shell's open editors: each tab is an environment (its Configure and
// Apps layers), one of its revisions or a comparison of two. Pure helpers, so
// the tab rules are testable on their own.

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
