// The shell's activity bar: one button per side view, the editor's own first,
// then a host's (see `SideViewContribution`), then a host's workbench views
// (see `WorkbenchViewContribution`), which take over the main area, then a
// host's actions (see `ActivityActionContribution`), which run instead of
// opening a sidebar; those aligned to the end sit at the bottom.

import {
  cn,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@open-lakehouse/ui-kit";
import { Files, History, Settings } from "lucide-react";
import type { ReactNode } from "react";
import type {
  ActivityActionContribution,
  SideViewContribution,
  WorkbenchViewContribution,
} from "./contributions.js";

/** A side view's id: the editor's, or a host's `SideViewContribution.id`. */
export type SideView = string;

/** The editor's own side views. */
export const BUILTIN_SIDE_VIEWS = ["explorer", "history", "settings"] as const;

/** A button in the activity bar: a side view, or (with `run`) a host's action. */
export interface ActivityItem {
  id: SideView;
  label: string;
  icon: ReactNode;
  align?: "start" | "end";
  /** An action: runs on a click instead of opening a sidebar view. */
  run?: () => void;
  /** A workbench view: takes over the main area instead of the sidebar. */
  workbench?: boolean;
}

/** What the activity bar lists: the editor's side views (history only with
 *  revisions), then the host's, its workbench views, then its actions. */
export function sideViewItems(
  history: boolean,
  extra: readonly SideViewContribution[] = [],
  actions: readonly ActivityActionContribution[] = [],
  workbench: readonly WorkbenchViewContribution[] = [],
): ActivityItem[] {
  return [
    { id: "explorer", label: "Environments", icon: <Files /> },
    ...(history
      ? [{ id: "history", label: "History", icon: <History /> }]
      : []),
    { id: "settings", label: "Workspace settings", icon: <Settings /> },
    ...extra.map(({ id, label, icon, align }) => ({ id, label, icon, align })),
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

export function ActivityBar({
  view,
  onView,
  problemCount,
  history,
  extra,
  actions,
  workbench,
}: {
  /** The highlighted item: the open side view, or the active workbench
   *  view. */
  view: SideView | undefined;
  onView: (view: SideView) => void;
  problemCount: number;
  /** Whether to offer the revision history (the store keeps revisions). */
  history: boolean;
  /** A host's side views. */
  extra?: readonly SideViewContribution[];
  /** A host's actions. */
  actions?: readonly ActivityActionContribution[];
  /** A host's workbench views. */
  workbench?: readonly WorkbenchViewContribution[];
}) {
  const items = sideViewItems(history, extra, actions, workbench);
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
          {item.id === "explorer" && problemCount > 0 && (
            <span className="absolute right-1 top-1 h-2 w-2 rounded-full bg-destructive" />
          )}
        </button>
      </TooltipTrigger>
      <TooltipContent side="right">{item.label}</TooltipContent>
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
