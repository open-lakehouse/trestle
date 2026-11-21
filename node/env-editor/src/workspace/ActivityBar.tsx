// The shell's navigation, in two places. The sidebar's own views (the
// editor's, then a host's, see `SideViewContribution`) are tabs at the top of
// the sidebar (`SidebarTabs`). The activity bar, the rail left of it, moves
// between experiences: the editor, a host's workbench views (see
// `WorkbenchViewContribution`), which take over the main area, and a host's
// actions (see `ActivityActionContribution`), which run instead; those aligned
// to the end sit at the bottom. Without workbench views or actions there is
// nothing to move between, and the rail isn't drawn.

import {
  cn,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@open-lakehouse/ui-kit";
import { Files, History, Settings, Workflow } from "lucide-react";
import type { ReactNode } from "react";
import { formatShortcut } from "../commands/registry.js";
import type {
  ActivityActionContribution,
  SideViewContribution,
  WorkbenchViewContribution,
} from "./contributions.js";
import { EDITOR_VIEW } from "./workbench.js";

/** A side view's id: the editor's, or a host's `SideViewContribution.id`. */
export type SideView = string;

/** The editor's own side views. */
export const BUILTIN_SIDE_VIEWS = ["explorer", "history", "settings"] as const;

/** A sidebar tab, an activity bar button or (with `run`) a host's action. */
export interface ActivityItem {
  id: SideView;
  label: string;
  icon: ReactNode;
  align?: "start" | "end";
  /** An action: runs on a click instead of opening a sidebar view. */
  run?: () => void;
  /** A workbench view: takes over the main area instead of the sidebar. */
  workbench?: boolean;
  /** The key that shows it (a `Command.shortcut`). */
  shortcut?: string;
}

/** Everything the shell navigates to, for its commands: the editor's side
 *  views (history only with revisions), then the host's, its workbench views,
 *  then its actions. */
export function sideViewItems(
  history: boolean,
  extra: readonly SideViewContribution[] = [],
  actions: readonly ActivityActionContribution[] = [],
  workbench: readonly WorkbenchViewContribution[] = [],
): ActivityItem[] {
  return [
    ...sidebarTabItems(history, extra),
    ...workbench.map(({ id, label, icon, align }) => ({
      id,
      label,
      icon,
      align,
      workbench: true,
    })),
    ...actions.map(({ id, label, icon, align, run }) => ({
      id,
      label,
      icon,
      align,
      run,
    })),
  ];
}

/** The sidebar's tabs: the editor's views (history only with revisions),
 *  then the host's. */
export function sidebarTabItems(
  history: boolean,
  extra: readonly SideViewContribution[] = [],
): ActivityItem[] {
  return [
    {
      id: "explorer",
      label: "Environments",
      icon: <Files />,
      shortcut: "mod+shift+e",
    },
    ...(history
      ? [
          {
            id: "history",
            label: "History",
            icon: <History />,
            shortcut: "mod+shift+h",
          },
        ]
      : []),
    { id: "settings", label: "Workspace settings", icon: <Settings /> },
    ...extra.map(({ id, label, icon, align }) => ({ id, label, icon, align })),
  ];
}

/** The activity bar's buttons: the editor, then the host's workbench views and
 *  actions. None when the host gives neither. */
export function activityItems(
  actions: readonly ActivityActionContribution[] = [],
  workbench: readonly WorkbenchViewContribution[] = [],
): ActivityItem[] {
  if (actions.length === 0 && workbench.length === 0) return [];
  return [
    { id: EDITOR_VIEW, label: "Editor", icon: <Workflow /> },
    ...sideViewItems(false, [], actions, workbench).filter(
      (i) => i.workbench || i.run,
    ),
  ];
}

/** An item's tooltip: its label, and its shortcut where it has one. */
export function itemTitle(item: Pick<ActivityItem, "label" | "shortcut">) {
  return item.shortcut
    ? `${item.label} (${formatShortcut(item.shortcut)})`
    : item.label;
}

export function ActivityBar({
  view,
  onView,
  problems = false,
  actions,
  workbench,
}: {
  /** The highlighted item: the active workbench view, or `EDITOR_VIEW`. */
  view: string | undefined;
  onView: (view: string) => void;
  /** Mark the editor's button: there are problems the sidebar doesn't show
   *  right now. */
  problems?: boolean;
  /** A host's actions. */
  actions?: readonly ActivityActionContribution[];
  /** A host's workbench views. */
  workbench?: readonly WorkbenchViewContribution[];
}) {
  const items = activityItems(actions, workbench);
  if (items.length === 0) return null;
  const button = (item: ActivityItem) => (
    <Tooltip key={item.id}>
      <TooltipTrigger asChild>
        <button
          type="button"
          aria-label={item.label}
          aria-pressed={item.run ? undefined : view === item.id}
          onClick={() => (item.run ? item.run() : onView(item.id))}
          className={cn(
            "relative flex h-10 w-10 items-center justify-center rounded-md text-muted-foreground transition-colors hover:text-foreground [&_svg]:h-5 [&_svg]:w-5",
            view === item.id &&
              "text-foreground before:absolute before:inset-y-2 before:-left-1 before:w-0.5 before:rounded-full before:bg-primary",
          )}
        >
          {item.icon}
          {item.id === EDITOR_VIEW && problems && (
            <span className="absolute right-1 top-1 h-2 w-2 rounded-full bg-destructive" />
          )}
        </button>
      </TooltipTrigger>
      <TooltipContent side="right">{itemTitle(item)}</TooltipContent>
    </Tooltip>
  );
  const end = items.filter((i) => i.align === "end");
  return (
    <nav
      aria-label="Views"
      className="flex w-12 shrink-0 flex-col items-center gap-1 border-r bg-activitybar py-2"
    >
      {items.filter((i) => i.align !== "end").map(button)}
      {end.length > 0 && (
        <div className="mt-auto flex flex-col items-center gap-1">
          {end.map(button)}
        </div>
      )}
    </nav>
  );
}
