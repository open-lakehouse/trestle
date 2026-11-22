// What a host adds to the workspace shell: commands, extra side views,
// workbench views that take over the main area, viewers for files and diffs,
// and per environment a run status with hover-card details, a run action,
// menu actions, more ways to commit, sub-tabs after the editor's own, each
// service's state on the canvas, and panels: tabs of the bottom panel, after
// Problems, in place of the static Run output. The editor only renders what
// it is given and learns nothing about how the host runs anything
// (containers, remote branches, ...). A host that contributes nothing gets the
// shell as it always was.
//
// Contributions are plain data plus callbacks, so a host can derive them from
// its own cache (a query cache, a store) and either re-render the provider
// with a new object or call `subscribe`'s listener when the cache changes.

import {
  createContext,
  type ReactNode,
  useContext,
  useEffect,
  useMemo,
  useState,
} from "react";
import type { FileSource } from "../artifacts/fileSource.js";
import type { Command } from "../commands/registry.js";
import type { NodeStatus, RunState } from "../graph/runState.js";
import { PROBLEMS_PANEL, RUN_OUTPUT_PANEL } from "./bottomPanel.js";
import type { RevisionSummary } from "./store.js";

export type { NodeStatus, RunState } from "../graph/runState.js";

/** An environment's run state: a dot on its tree row and its tab, and its
 *  hover card. */
export interface EnvironmentStatus {
  state: RunState;
  /** Short text for the state (e.g. "running 3/3"); defaults to its name. */
  label?: string;
  /** The revision running, where the host tracks revisions. */
  revision?: number;
  /** Where it runs, e.g. `local slot 0` or a remote branch's name. */
  placement?: string;
  /** More detail, shown in tooltips. */
  detail?: string;
  /** Facts for its hover card, e.g. a remote target or the gateway's URL. */
  details?: EnvironmentDetail[];
}

/** One fact in an environment's hover card: `label value`, optionally
 *  copyable, and optionally a button that runs something (e.g. opens a
 *  sub-tab). */
export interface EnvironmentDetail {
  label: string;
  value: string;
  /** Offer a button that copies `value`. */
  copy?: boolean;
  /** Tooltip; defaults to `value`. */
  title?: string;
  /** Drawn before the label, e.g. the provider's logo. */
  icon?: ReactNode;
  /** Makes the fact a button that runs this when clicked (and closes the
   *  card), e.g. to open the environment's Configure tab. Its copy button, if
   *  any, stays beside it. */
  run?: () => void;
}

/** Something a host offers to do with an environment: its run action (an
 *  icon button) or an entry of its ⋯ menus. */
export interface HostAction {
  /** Unique among the actions it is listed with. */
  id: string;
  label: string;
  icon?: ReactNode;
  /** Defaults to `true`. */
  enabled?: boolean;
  /** Why it is disabled, shown with it. */
  reason?: string;
  /** Defaults to `"default"`. */
  variant?: "primary" | "default" | "destructive";
  run: () => void;
}

/** An entry of the Commit button's menu, e.g. "Commit and start": it commits
 *  the draft, then runs with the new revision. */
export interface CommitAction {
  /** Unique among the commit actions. */
  id: string;
  label: string;
  icon?: ReactNode;
  /** Defaults to `true`. */
  enabled?: boolean;
  /** Why it is disabled, shown with it. */
  reason?: string;
  /** Runs after a successful commit, with the revision it made. */
  run: (revision: RevisionSummary) => void;
}

/** A sub-tab of an environment's tab, after the editor's own Diagram, Files
 *  and Changes. */
export interface EnvironmentTab {
  /** Unique per environment; `"diagram"`, `"files"` and `"changes"` (and
   *  the older `"design"`) are the editor's. */
  id: string;
  label: string;
  icon?: ReactNode;
  /** A disabled tab is listed but can't be picked. */
  enabled: boolean;
  /** Why it is disabled, e.g. "dev isn't running". */
  reason?: string;
  /** The tab's body. The pane gives it `min-h-0 flex-1`, so a body that
   *  scrolls renders its own `overflow-auto` (or the like). */
  render: () => ReactNode;
}

/** A view in the activity bar, after the editor's own. */
export interface SideViewContribution {
  /** Unique among the side views; the editor's are `explorer`, `history`,
   *  `modules` and `settings`. */
  id: string;
  label: string;
  icon: ReactNode;
  /** `"end"` puts it at the bottom of the activity bar (e.g. app settings). */
  align?: "start" | "end";
  /** The sidebar's content while the view is open. */
  render: () => ReactNode;
}

/** A view that takes over the shell's main area: an activity-bar item that,
 *  while active, shows `render` in place of the editor tabs (which stay
 *  mounted, hidden, so the diagram and viewers keep their state) and
 *  `sidebar`, if given, in place of the sidebar. The ⌘K palette, bottom
 *  panel and status bar stay. Open one from code with `useWorkbench`. */
export interface WorkbenchViewContribution {
  /** Unique among the activity bar's items; `"editor"` is reserved. */
  id: string;
  label: string;
  icon: ReactNode;
  /** `"end"` puts it at the bottom of the activity bar. */
  align?: "start" | "end";
  /** The sidebar's content while the view is active; without it the
   *  sidebar keeps its side view. */
  sidebar?: () => ReactNode;
  /** The main area's content while the view is active. */
  render: () => ReactNode;
}

/** A button in the activity bar that runs instead of opening a sidebar view: e.g.
 *  a host's settings, opened in a dialog over the workspace, so the editor keeps
 *  the window. */
export interface ActivityActionContribution {
  /** Unique among the activity bar's items. */
  id: string;
  label: string;
  icon: ReactNode;
  /** `"end"` puts it at the bottom of the activity bar. */
  align?: "start" | "end";
  run: () => void;
}

/** A host's tab of the bottom panel, after Problems. Without any, the bottom
 *  panel shows the static Run output. Open one from code with
 *  `useBottomPanel`. */
export interface PanelContribution {
  /** Unique among the host's panels; `"problems"` is the editor's. */
  id: string;
  label: string;
  icon?: ReactNode;
  /** The panel's content for the active environment (`undefined` when no
   *  environment is open). */
  render: (envId: string | undefined) => ReactNode;
}

/** The deprecated single panel: a `PanelContribution` whose `id` may be left
 *  out, and then is `"output"`. */
export type LegacyPanelContribution = Omit<PanelContribution, "id"> & {
  id?: string;
};

/** The host's panels in tab order: `panels`, then the deprecated `panel`. A
 *  panel whose id is `"problems"`, or repeats an earlier one, is dropped. */
export function hostPanels(
  contributions: EditorContributions | undefined,
): PanelContribution[] {
  const legacy = contributions?.panel;
  const all: PanelContribution[] = [
    ...(contributions?.panels ?? []),
    ...(legacy ? [{ ...legacy, id: legacy.id ?? RUN_OUTPUT_PANEL }] : []),
  ];
  const seen = new Set<string>([PROBLEMS_PANEL]);
  return all.filter((p) => {
    if (seen.has(p.id)) return false;
    seen.add(p.id);
    return true;
  });
}

/** What a host's file viewer gets: one rendered file of an environment. */
export interface FileViewerProps {
  envId: string;
  path: string;
  /** The file's contents (see `FileSource`). */
  read: FileSource["read"];
}

/** What a host's diff viewer gets: one file before and after. `original`
 *  is absent for an added file, `modified` for a removed one. */
export interface DiffViewerProps {
  envId: string;
  path: string;
  original?: string;
  modified?: string;
}

/** What a host knows about each environment, by the workspace's env id. */
export interface EnvironmentContributions {
  status?: (envId: string) => EnvironmentStatus | undefined;
  /** The one action drawn as an icon button on the environment's tree row
   *  and at the end of its sub-tabs: typically Start, or Stop while it runs.
   *  Its icon is drawn alone, its label is the tooltip. */
  runAction?: (envId: string) => HostAction | undefined;
  /** Everything else the host offers to do with it, in its ⋯ menus (the
   *  tree row's, after Rename, Duplicate and Remove, and the sub-tabs').
   *  An action with the `runAction`'s id is left out of them. */
  actions?: (envId: string) => HostAction[];
  /** More ways to commit, in the Commit button's menu. */
  commitActions?: (envId: string) => CommitAction[];
  tabs?: (envId: string) => EnvironmentTab[];
  /** A service's state, by catalog module id. */
  nodeStatus?: (envId: string, moduleId: string) => NodeStatus | undefined;
  /** Call `onChange` whenever any of the above would answer differently.
   *  Returns an unsubscribe function. */
  subscribe?: (onChange: () => void) => () => void;
}

/** Everything a host contributes to the workspace shell. */
export interface EditorContributions {
  /** Commands for the ⌘K palette, under their own sections (e.g.
   *  "Environment", "App"). Their key bindings take precedence over the
   *  editor's (see `EDITOR_SHORTCUTS`). */
  commands?: Command[];
  sideViews?: SideViewContribution[];
  /** Activity-bar buttons that run an action (also listed in the ⌘K palette). */
  activityActions?: ActivityActionContribution[];
  /** Views that take over the main area (e.g. a host's Apps), each with an
   *  activity-bar item. */
  workbenchViews?: WorkbenchViewContribution[];
  environment?: EnvironmentContributions;
  /** Tabs of the bottom panel, after Problems, in place of the static Run
   *  output. Open one from code with `useBottomPanel().open(id)`. */
  panels?: PanelContribution[];
  /** @deprecated Use `panels`. One panel, with the id `"output"` unless it
   *  has one. */
  panel?: LegacyPanelContribution;
  /** Renders a rendered file read-only (e.g. Monaco); the editor's
   *  syntax-highlighted viewer otherwise. */
  fileViewer?: (props: FileViewerProps) => ReactNode;
  /** Renders one file's change (e.g. Monaco's diff editor); the editor's
   *  unified line diff otherwise. */
  diffViewer?: (props: DiffViewerProps) => ReactNode;
}

interface ContributionsValue {
  contributions: EditorContributions | undefined;
  /** Bumped on every `environment.subscribe` notification. */
  version: number;
}

const ContributionsContext = createContext<ContributionsValue>({
  contributions: undefined,
  version: 0,
});

export interface ContributionsProviderProps {
  contributions?: EditorContributions;
  children: ReactNode;
}

/** Hands a host's contributions to the workspace shell beneath it. */
export function ContributionsProvider({
  contributions,
  children,
}: ContributionsProviderProps) {
  const [version, setVersion] = useState(0);
  const subscribe = contributions?.environment?.subscribe;
  useEffect(() => {
    if (!subscribe) return;
    return subscribe(() => setVersion((v) => v + 1));
  }, [subscribe]);
  const value = useMemo(
    () => ({ contributions, version }),
    [contributions, version],
  );
  return (
    <ContributionsContext.Provider value={value}>
      {children}
    </ContributionsContext.Provider>
  );
}

/** The host's contributions, and a `version` that changes when they do. */
export function useContributions(): ContributionsValue {
  return useContext(ContributionsContext);
}
