// An environment tab's chrome: a header with the environment's name, what the
// host reports about it (state, revision, placement, details) and the host's
// actions; under it the sub-tabs: the editor's Diagram, Files and Changes,
// then the host's. See `editors.ts` for the sub-tab rules and
// `contributions.tsx` for what a host gives.

import {
  Button,
  cn,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@open-lakehouse/ui-kit";
import { FileText, GitCompare, Workflow } from "lucide-react";
import type { ReactNode } from "react";
import { CopyButton } from "../artifacts/CodeViewer.js";
import { RunStateDot } from "../graph/RunStateDot.js";
import { runStateLabel } from "../graph/runState.js";
import {
  type EnvironmentDetail,
  type EnvironmentStatus,
  type HostAction,
  useContributions,
} from "./contributions.js";
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

export type { SubTab } from "./editors.js";

/** The Diagram sub-tab's id (it was Design before 0.0.7). */
export const DESIGN_TAB = DIAGRAM_TAB;

const BUILTIN_ICONS: Record<string, ReactNode> = {
  [DIAGRAM_TAB]: <Workflow />,
  [FILES_TAB]: <FileText />,
  [CHANGES_TAB]: <GitCompare />,
};

/** The header over an environment tab: its name and status, the host's
 *  details, then the host's actions. */
export function EnvHeader({
  name,
  status,
  planStatus,
  details = [],
  actions,
}: {
  name?: string;
  status: EnvironmentStatus | undefined;
  /** Drawn in place of a host's status, e.g. whether the environment plans. */
  planStatus?: ReactNode;
  /** The editor's own facts (e.g. the gateway URL), shown when the host
   *  reports none. */
  details?: readonly EnvironmentDetail[];
  actions: readonly HostAction[];
}) {
  const facts = status?.details ?? details;
  return (
    <div className="flex h-10 shrink-0 items-center gap-3 border-b px-3 text-xs">
      {name && (
        <span className="inline-flex min-w-0 items-center gap-1.5 text-sm font-medium">
          {!status && planStatus}
          <span className="truncate">{name}</span>
        </span>
      )}
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
      {facts.map((fact) => (
        <Detail key={fact.label} detail={fact} />
      ))}
      <div className="ml-auto flex shrink-0 items-center gap-1.5">
        {actions.map((action) => (
          <ActionButton key={action.id} action={action} />
        ))}
      </div>
    </div>
  );
}

function Detail({ detail }: { detail: EnvironmentDetail }) {
  return (
    <span
      className="inline-flex min-w-0 items-center gap-1 text-muted-foreground"
      title={detail.title ?? detail.value}
    >
      <span>{detail.label}</span>
      <span className="truncate font-mono text-foreground">{detail.value}</span>
      {detail.copy && (
        <CopyButton
          text={detail.value}
          label={`Copy ${detail.label}`}
          className="h-5 w-5 [&_svg]:h-3 [&_svg]:w-3"
        />
      )}
    </span>
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

/** An environment tab's sub-tabs. A disabled tab says why and can't be
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
            {tab.icon ?? BUILTIN_ICONS[tab.id]}
            {tab.label}
          </button>
        );
      })}
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
  /** The environment's name, for its header. */
  name: string;
  view: EnvView;
  onView: (view: EnvView) => void;
  /** Whether there is a Changes tab, and whether it has a revision to show. */
  revisions: ChangesAvailability;
  /** The environment's plan state, in the header when the host reports no
   *  status. */
  planStatus?: ReactNode;
  /** The editor's own header facts (e.g. the gateway URL). */
  details?: readonly EnvironmentDetail[];
  /** The editor's own sub-tabs' bodies. */
  diagram: ReactNode;
  files: ReactNode;
  changes: ReactNode;
}

/**
 * An environment tab: its header, its sub-tabs, then the active sub-tab's
 * body: `diagram`, `files`, `changes` or a host's tab.
 */
export function EnvironmentPane({
  envId,
  name,
  view,
  onView,
  revisions,
  planStatus,
  details,
  diagram,
  files,
  changes,
}: EnvironmentPaneProps) {
  const env = useContributions().contributions?.environment;
  const status = env?.status?.(envId);
  const actions = env?.actions?.(envId) ?? [];
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
      <EnvHeader
        name={name}
        status={status}
        planStatus={planStatus}
        details={details}
        actions={actions}
      />
      <EnvSubTabs
        tabs={tabs}
        active={subTab}
        onSelect={(id) => onView({ ...view, subTab: id })}
      />
      <div className="min-h-0 flex-1">{body}</div>
    </div>
  );
}
