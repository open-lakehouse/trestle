// An environment tab's chrome: its sub-tabs (the editor's Diagram, Files and
// Changes, then the host's) and, at their end, the host's run action and a ⋯
// menu of its other actions. What the host reports about the environment is
// in its hover card (`envCard.tsx`), on its editor tab and tree row. See
// `editors.ts` for the sub-tab rules and `contributions.tsx` for what a host
// gives.

import { cn } from "@open-lakehouse/ui-kit";
import { FileText, GitCompare, Workflow } from "lucide-react";
import type { ReactNode } from "react";
import { useContributions } from "./contributions.js";
import {
  CHANGES_TAB,
  type ChangesAvailability,
  DIAGRAM_TAB,
  type EnvView,
  envSubTabs,
  FILES_TAB,
  resolveSubTab,
  type SubTab,
} from "./editors.js";
import { ActionsMenu, RunActionButton, useEnvActions } from "./envActions.js";

export type { SubTab } from "./editors.js";

/** The Diagram sub-tab's id (it was Design before 0.0.7). */
export const DESIGN_TAB = DIAGRAM_TAB;

const BUILTIN_ICONS: Record<string, ReactNode> = {
  [DIAGRAM_TAB]: <Workflow />,
  [FILES_TAB]: <FileText />,
  [CHANGES_TAB]: <GitCompare />,
};

/** An environment tab's sub-tabs, `trailing` at their end. A disabled tab
 *  says why and can't be picked. */
export function EnvSubTabs({
  tabs,
  active,
  onSelect,
  trailing,
}: {
  tabs: readonly SubTab[];
  active: string;
  onSelect: (id: string) => void;
  /** Drawn at the end of the row, e.g. the environment's run action. */
  trailing?: ReactNode;
}) {
  return (
    <div className="flex h-8 shrink-0 items-stretch border-b px-2">
      <div
        role="tablist"
        aria-label="Environment views"
        className="flex min-w-0 items-stretch gap-1 overflow-x-auto"
      >
        {tabs.map((tab) => {
          const selected = tab.id === active;
          return (
            <button
              key={tab.id}
              type="button"
              role="tab"
              aria-selected={selected}
              aria-disabled={!tab.enabled || undefined}
              title={tab.enabled ? undefined : tab.reason}
              onClick={() => {
                if (tab.enabled) onSelect(tab.id);
              }}
              className={cn(
                "inline-flex items-center gap-1.5 border-b-2 px-2 text-xs [&_svg]:h-3.5 [&_svg]:w-3.5",
                selected
                  ? "border-primary text-foreground"
                  : "border-transparent text-muted-foreground",
                tab.enabled
                  ? !selected && "hover:text-foreground"
                  : "cursor-not-allowed opacity-50",
              )}
            >
              {tab.icon ?? BUILTIN_ICONS[tab.id]}
              {tab.label}
            </button>
          );
        })}
      </div>
      {trailing && (
        <div className="ml-auto flex shrink-0 items-center gap-0.5 pl-2">
          {trailing}
        </div>
      )}
    </div>
  );
}

/** The Diagram sub-tab, as listed first. */
export const designTab: SubTab = {
  id: DIAGRAM_TAB,
  label: "Diagram",
  icon: <Workflow />,
  enabled: true,
};

/** The body of a tab that is selected but not enabled. */
export function UnavailableTab({
  label,
  reason,
}: {
  label: string;
  reason?: string;
}) {
  return (
    <div className="flex h-full items-center justify-center p-6 text-sm text-muted-foreground">
      {reason
        ? `${label} is unavailable: ${reason}`
        : `${label} is unavailable.`}
    </div>
  );
}

/** The sub-tabs of environment `envId`: the editor's, then the host's. */
export function useEnvSubTabs(
  envId: string | undefined,
  revisions: ChangesAvailability,
): SubTab[] {
  const env = useContributions().contributions?.environment;
  return envSubTabs((envId && env?.tabs?.(envId)) || [], revisions);
}

export interface EnvironmentPaneProps {
  envId: string;
  /** The environment's name, for its menu's label. */
  name: string;
  view: EnvView;
  onView: (view: EnvView) => void;
  /** Whether there is a Changes tab, and whether it has a revision to show. */
  revisions: ChangesAvailability;
  /** The editor's own sub-tabs' bodies. */
  diagram: ReactNode;
  files: ReactNode;
  changes: ReactNode;
}

/**
 * An environment tab: its sub-tabs with the host's run action and ⋯ menu at
 * their end, then the active sub-tab's body: `diagram`, `files`, `changes`
 * or a host's tab.
 */
export function EnvironmentPane({
  envId,
  name,
  view,
  onView,
  revisions,
  diagram,
  files,
  changes,
}: EnvironmentPaneProps) {
  const env = useContributions().contributions?.environment;
  const { run, menu } = useEnvActions(envId);
  const hostTabs = env?.tabs?.(envId) ?? [];
  const tabs = envSubTabs(hostTabs, revisions);
  const subTab = resolveSubTab(view, tabs);
  const active = tabs.find((t) => t.id === subTab);
  const host = hostTabs.find((t) => t.id === subTab);

  let body: ReactNode;
  if (active && !active.enabled) {
    body = <UnavailableTab label={active.label} reason={active.reason} />;
  } else if (host) {
    body = host.render();
  } else if (subTab === FILES_TAB) {
    body = files;
  } else if (subTab === CHANGES_TAB) {
    body = changes;
  } else {
    body = diagram;
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <EnvSubTabs
        tabs={tabs}
        active={subTab}
        onSelect={(id) => onView({ ...view, subTab: id })}
        trailing={
          (run || menu.length > 0) && (
            <>
              {run && <RunActionButton action={run} />}
              <ActionsMenu label={`More actions for ${name}`} actions={menu} />
            </>
          )
        }
      />
      <div className="min-h-0 flex-1">{body}</div>
    </div>
  );
}
