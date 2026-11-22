// An environment's hover card: what the host reports about it (its state,
// revision, placement and details, e.g. its remote branch, its model provider
// and its gateway's URL), shown while the pointer rests on its editor tab or,
// a little longer, on its tree row. See `contributions.tsx` for what a host
// gives.

import {
  cn,
  HoverCard,
  HoverCardContent,
  HoverCardTrigger,
} from "@open-lakehouse/ui-kit";
import { type ReactElement, type ReactNode, useState } from "react";
import { CopyButton } from "../artifacts/CodeViewer.js";
import { RunStateDot } from "../graph/RunStateDot.js";
import { runStateLabel } from "../graph/runState.js";
import {
  type EnvironmentDetail,
  type EnvironmentStatus,
  useContributions,
} from "./contributions.js";

export interface EnvCardProps {
  name: string;
  status: EnvironmentStatus | undefined;
  /** Drawn before the name, e.g. whether the environment plans. */
  planStatus?: ReactNode;
  /** The editor's own facts (e.g. the gateway URL), shown when the host
   *  reports none. */
  details?: readonly EnvironmentDetail[];
  /** A last line in small print, e.g. its slot's host ports. */
  footnote?: ReactNode;
  /** Called after a detail's `run`, e.g. to close the card. */
  onRun?: () => void;
}

/** The card's body: the name and state, the revision and placement, then
 *  one row per detail. */
export function EnvCard({
  name,
  status,
  planStatus,
  details = [],
  footnote,
  onRun,
}: EnvCardProps) {
  const facts = status?.details ?? details;
  const where = [
    status?.revision !== undefined ? `rev ${status.revision}` : undefined,
    status?.placement,
  ].filter(Boolean);
  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center gap-2">
        {planStatus}
        <span className="min-w-0 truncate text-sm font-medium">{name}</span>
        {status && (
          <span className="ml-auto inline-flex shrink-0 items-center gap-1.5 text-muted-foreground">
            <RunStateDot status={status} />
            {runStateLabel(status)}
          </span>
        )}
      </div>
      {where.length > 0 && (
        <div className="truncate font-mono text-[11px] text-muted-foreground">
          {where.join(" · ")}
        </div>
      )}
      {status?.detail && (
        <p className="text-[11px] text-muted-foreground">{status.detail}</p>
      )}
      {facts.length > 0 && (
        <dl className="grid grid-cols-[auto_minmax(0,1fr)] items-center gap-x-3 gap-y-0.5">
          {facts.map((fact) => (
            <DetailRow key={fact.label} detail={fact} onRun={onRun} />
          ))}
        </dl>
      )}
      {footnote && (
        <div className="text-[11px] text-muted-foreground">{footnote}</div>
      )}
    </div>
  );
}

/** One detail: its icon and label, then its value; a button where it runs
 *  something, with its copy button beside it. */
function DetailRow({
  detail,
  onRun,
}: {
  detail: EnvironmentDetail;
  onRun?: () => void;
}) {
  const { icon, label, value, copy, title, run } = detail;
  const valueText = (
    <span className="truncate font-mono text-foreground">{value}</span>
  );
  return (
    <>
      <dt className="inline-flex items-center gap-1.5 text-muted-foreground">
        <span className="inline-flex h-3.5 w-3.5 shrink-0 items-center justify-center [&_svg]:h-3.5 [&_svg]:w-3.5">
          {icon}
        </span>
        {label}
      </dt>
      <dd className="flex min-w-0 items-center gap-0.5">
        {run ? (
          <button
            type="button"
            title={title ?? value}
            onClick={() => {
              run();
              onRun?.();
            }}
            className="-mx-1 flex min-w-0 items-center rounded px-1 py-0.5 text-left hover:bg-accent"
          >
            {valueText}
          </button>
        ) : (
          <span className="flex min-w-0 py-0.5" title={title ?? value}>
            {valueText}
          </span>
        )}
        {copy && (
          <CopyButton
            text={value}
            label={`Copy ${label}`}
            className="h-5 w-5 shrink-0 [&_svg]:h-3 [&_svg]:w-3"
          />
        )}
      </dd>
    </>
  );
}

export interface EnvHoverCardProps
  extends Omit<EnvCardProps, "status" | "onRun"> {
  envId: string;
  /** How long the pointer rests before the card opens, in ms. */
  openDelay?: number;
  side?: "top" | "right" | "bottom" | "left";
  /** Keeps it closed, e.g. while the trigger's menu is open. */
  disabled?: boolean;
  /** Asked before it opens: `true` keeps it closed (e.g. while the pointer
   *  is on the trigger's own buttons). Read when it would open, so the
   *  trigger needn't re-render to change it. */
  blocked?: () => boolean;
  /** Receives a function that closes the card. */
  closeRef?: { current: (() => void) | null };
  /** The trigger: one element that takes a ref and pointer handlers. */
  children: ReactElement;
}

/** Shows environment `envId`'s card while the pointer rests on `children`. */
export function EnvHoverCard({
  envId,
  openDelay = 500,
  side = "bottom",
  disabled,
  blocked,
  closeRef,
  children,
  ...card
}: EnvHoverCardProps) {
  const status = useContributions().contributions?.environment?.status?.(envId);
  const [open, setOpen] = useState(false);
  if (closeRef) closeRef.current = () => setOpen(false);
  return (
    <HoverCard
      open={open && !disabled}
      onOpenChange={(next) => setOpen(next && !blocked?.())}
      openDelay={openDelay}
      closeDelay={100}
    >
      <HoverCardTrigger asChild>{children}</HoverCardTrigger>
      <HoverCardContent
        side={side}
        aria-label={`About ${card.name}`}
        className={cn(side === "right" && "ml-1")}
      >
        <EnvCard {...card} status={status} onRun={() => setOpen(false)} />
      </HoverCardContent>
    </HoverCard>
  );
}
