// An environment tab's host chrome: a header with what the host reports
// about the environment (state, revision, placement) and the actions it
// offers, and the sub-tabs next to the editor's own Design. Drawn only when a
// host contributes them (see `contributions.tsx`).

import {
  Button,
  cn,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@open-lakehouse/ui-kit";
import { Workflow } from "lucide-react";
import type { ReactNode } from "react";
import { RunStateDot } from "../graph/RunStateDot.js";
import { runStateLabel } from "../graph/runState.js";
import {
  type EnvironmentStatus,
  type HostAction,
  useContributions,
} from "./contributions.js";

/** The editor's own sub-tab. */
export const DESIGN_TAB = "design";

/** The header over an environment tab: its status, then the host's actions. */
export function EnvHeader({
  status,
  actions,
}: {
  status: EnvironmentStatus | undefined;
  actions: readonly HostAction[];
}) {
  return (
    <div className="flex h-9 shrink-0 items-center gap-3 border-b px-3 text-xs">
      {status && (
        <>
          <span
            className="inline-flex items-center gap-1.5 font-medium"
            title={status.detail}
          >
            <RunStateDot status={status} />
            {runStateLabel(status)}
          </span>
          {status.revision !== undefined && (
            <span className="font-mono text-muted-foreground">
              rev {status.revision}
            </span>
          )}
          {status.placement && (
            <span className="truncate text-muted-foreground">
              {status.placement}
            </span>
          )}
        </>
      )}
      <div className="ml-auto flex items-center gap-1.5">
        {actions.map((action) => (
          <ActionButton key={action.id} action={action} />
        ))}
      </div>
    </div>
  );
}

function ActionButton({ action }: { action: HostAction }) {
  const enabled = action.enabled ?? true;
  const button = (
    <Button
      type="button"
      size="sm"
      variant={
        action.variant === "primary"
          ? "default"
          : action.variant === "destructive"
            ? "destructive"
            : "outline"
      }
      disabled={!enabled}
      onClick={action.run}
      className="h-6 gap-1 px-2 text-xs [&_svg]:h-3.5 [&_svg]:w-3.5"
    >
      {action.icon}
      {action.label}
    </Button>
  );
  if (enabled || !action.reason) return button;
  // A disabled button gets no pointer events: the wrapper carries the tooltip.
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        {/* biome-ignore lint/a11y/noNoninteractiveTabindex: lets keyboard users reach the reason a disabled action can't run. */}
        <span tabIndex={0} title={action.reason}>
          {button}
        </span>
      </TooltipTrigger>
      <TooltipContent>{action.reason}</TooltipContent>
    </Tooltip>
  );
}

/** One sub-tab, as `EnvSubTabs` lists it. */
export interface SubTab {
  id: string;
  label: string;
  icon?: ReactNode;
  enabled: boolean;
  reason?: string;
}

/** Design, then the host's tabs. A disabled tab says why and can't be
 *  picked. */
export function EnvSubTabs({
  tabs,
  active,
  onSelect,
}: {
  tabs: readonly SubTab[];
  active: string;
  onSelect: (id: string) => void;
}) {
  return (
    <div
      role="tablist"
      aria-label="Environment views"
      className="flex h-8 shrink-0 items-stretch gap-1 border-b px-2"
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
            {tab.icon}
            {tab.label}
          </button>
        );
      })}
    </div>
  );
}

/** The editor's own sub-tab, as listed first. */
export const designTab: SubTab = {
  id: DESIGN_TAB,
  label: "Design",
  icon: <Workflow />,
  enabled: true,
};

/** The body of a host tab that is selected but no longer enabled. */
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

/**
 * An environment's editor under the host's header and sub-tabs, when the host
 * contributes any: Design (`children`, the environment editor) first, then
 * the host's tabs. Without contributions it is the editor alone.
 */
export function EnvironmentPane({
  envId,
  subTab,
  onSubTab,
  children,
}: {
  envId: string;
  subTab: string;
  onSubTab: (id: string) => void;
  children: ReactNode;
}) {
  const env = useContributions().contributions?.environment;
  const status = env?.status?.(envId);
  const actions = env?.actions?.(envId) ?? [];
  const tabs = env?.tabs?.(envId) ?? [];
  if (!status && actions.length === 0 && tabs.length === 0) return children;
  const hostTab = tabs.find((t) => t.id === subTab);
  return (
    <div className="flex h-full min-h-0 flex-col">
      {(status || actions.length > 0) && (
        <EnvHeader status={status} actions={actions} />
      )}
      {tabs.length > 0 && (
        <EnvSubTabs
          tabs={[designTab, ...tabs]}
          active={hostTab ? subTab : DESIGN_TAB}
          onSelect={onSubTab}
        />
      )}
      <div className="min-h-0 flex-1">
        {!hostTab ? (
          children
        ) : hostTab.enabled ? (
          hostTab.render()
        ) : (
          <UnavailableTab label={hostTab.label} reason={hostTab.reason} />
        )}
      </div>
    </div>
  );
}
