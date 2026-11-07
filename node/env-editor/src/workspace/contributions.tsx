// What a host adds to the workspace shell: commands, extra side views, and per
// environment a run status, header actions, sub-tabs next to Design, each
// service's state on the canvas, and a panel in place of the static Run
// output. The editor only renders what it is given and learns nothing about
// how the host runs anything (containers, remote branches, ...). A host that
// contributes nothing gets the shell as it always was.
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
import type { Command } from "../commands/registry.js";
import type { NodeStatus, RunState } from "../graph/runState.js";

export type { NodeStatus, RunState } from "../graph/runState.js";

/** An environment's run state, for its tree row and its tab's header. */
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
}

/** A button a host offers, e.g. in an environment's header. */
export interface HostAction {
  /** Unique among the actions it is listed with. */
  id: string;
  label: string;
  icon?: ReactNode;
  /** Defaults to `true`. */
  enabled?: boolean;
  /** Why it is disabled, shown as its tooltip. */
  reason?: string;
  /** Defaults to `"default"`. */
  variant?: "primary" | "default" | "destructive";
  run: () => void;
}

/** A sub-tab of an environment's editor, after the editor's own Design. */
export interface EnvironmentTab {
  /** Unique per environment; `"design"` is the editor's. */
  id: string;
  label: string;
  icon?: ReactNode;
  /** A disabled tab is listed but can't be picked. */
  enabled: boolean;
  /** Why it is disabled, e.g. "dev isn't running". */
  reason?: string;
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

/** A host panel, in place of the bottom panel's static Run output. */
export interface PanelContribution {
  label: string;
  icon?: ReactNode;
  /** The panel's content for the active environment (`undefined` when no
   *  environment is open). */
  render: (envId: string | undefined) => ReactNode;
}

/** What a host knows about each environment, by the workspace's env id. */
export interface EnvironmentContributions {
  status?: (envId: string) => EnvironmentStatus | undefined;
  actions?: (envId: string) => HostAction[];
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
  environment?: EnvironmentContributions;
  panel?: PanelContribution;
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
