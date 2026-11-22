// An environment's status as one small icon, on its tree row, its editor tab
// and its card: whether it plans (or failed to, or clashes with another), what
// the host reports about it running (a ring circling while it runs, faster
// while it starts or stops), and a dot when its draft has changes no revision
// holds yet. The icons follow pipeline UIs: a check when ready, a cross when
// failed, a bang when degraded, a dashed ring while planning.

import { cn } from "@open-lakehouse/ui-kit";
import { type ReactNode, useEffect, useState } from "react";
import { type RunState, runStateTitle } from "../graph/runState.js";
import type { EnvironmentStatus } from "./contributions.js";
import type { EnvDoc } from "./model.js";
import { selectionDigest } from "./store.js";
import type { MemberPlan } from "./useMemberPlans.js";
import { useWorkspace } from "./useWorkspace.js";

/** Whether an environment's draft is in a revision: `uncommitted` (no
 *  revision yet), `changed` (differs from its last) or `committed`. */
export type DraftState = "uncommitted" | "changed" | "committed";

/** What the indicator draws. */
export type IndicatorKind =
  | "planning"
  | "plan-error"
  | "clash"
  | "ready"
  | RunState;

export interface Indicator {
  kind: IndicatorKind;
  /** Its draft has changes no revision holds. */
  dirty: boolean;
  /** Says what it shows, for its tooltip and accessible name. */
  title: string;
}

/** The indicator for an environment: a plan problem first, then what the
 *  host reports (the host's `stopped` is ready when it plans), then the plan.
 *  `plan` is undefined while it plans. */
export function environmentIndicator({
  plan,
  clashing,
  status,
  draft,
}: {
  plan: MemberPlan | undefined;
  clashing?: boolean;
  status?: EnvironmentStatus;
  draft?: DraftState;
}): Indicator {
  const dirty = draft === "uncommitted" || draft === "changed";
  const notes: string[] = [];
  let kind: IndicatorKind;
  if (plan?.error) {
    kind = "plan-error";
    notes.push(`doesn't plan: ${plan.error}`);
  } else if (status && status.state !== "stopped") {
    kind = status.state;
    notes.push(runStateTitle(status));
    if (clashing) notes.push("a host port clashes with another environment");
  } else if (clashing) {
    kind = "clash";
    notes.push("a host port clashes with another environment");
  } else if (!plan) {
    kind = "planning";
    notes.push("planning");
  } else {
    kind = "ready";
    notes.push(status ? runStateTitle(status) : "plans");
  }
  if (draft === "uncommitted") notes.push("not committed yet");
  if (draft === "changed") notes.push("the draft has uncommitted changes");
  return { kind, dirty, title: notes.join(" · ") };
}

/** A ring, partly open, circling: `fast` while starting or stopping. */
function Spinner({ tone, fast }: { tone: string; fast?: boolean }) {
  return (
    <svg
      viewBox="0 0 16 16"
      fill="none"
      aria-hidden="true"
      className={cn("h-full w-full motion-safe:animate-spin", tone)}
      style={{ animationDuration: fast ? "0.9s" : "2.4s" }}
    >
      <circle
        cx="8"
        cy="8"
        r="6"
        stroke="currentColor"
        strokeOpacity="0.25"
        strokeWidth="2"
      />
      <path
        d="M8 2a6 6 0 0 1 6 6"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
      />
    </svg>
  );
}

/** A filled disc with a glyph knocked out of it. */
function Disc({ tone, children }: { tone: string; children: ReactNode }) {
  return (
    <svg
      viewBox="0 0 16 16"
      aria-hidden="true"
      className={cn("h-full w-full", tone)}
    >
      <circle cx="8" cy="8" r="7" fill="currentColor" />
      <g
        stroke="hsl(var(--background))"
        strokeWidth="1.75"
        strokeLinecap="round"
        strokeLinejoin="round"
        fill="none"
      >
        {children}
      </g>
    </svg>
  );
}

function Glyph({ kind }: { kind: IndicatorKind }) {
  switch (kind) {
    case "running":
      return <Spinner tone="text-success" />;
    case "starting":
    case "stopping":
      return <Spinner tone="text-status-ready" fast />;
    case "failed":
    case "plan-error":
      return (
        <Disc tone="text-destructive">
          <path d="M5.5 5.5l5 5M10.5 5.5l-5 5" />
        </Disc>
      );
    case "degraded":
    case "clash":
      return (
        <Disc tone="text-status-ready">
          <path d="M8 4.5v4.25" />
          <path d="M8 11.25v.01" />
        </Disc>
      );
    case "ready":
      return (
        <Disc tone="text-success">
          <path d="M5 8.25l2 2 4-4.25" />
        </Disc>
      );
    case "planning":
      return (
        <svg
          viewBox="0 0 16 16"
          fill="none"
          aria-hidden="true"
          className="h-full w-full text-muted-foreground"
        >
          <circle
            cx="8"
            cy="8"
            r="6"
            stroke="currentColor"
            strokeWidth="2"
            strokeDasharray="2.6 2.1"
          />
        </svg>
      );
    default:
      // unknown, or the host's stopped without a plan to speak of.
      return (
        <svg
          viewBox="0 0 16 16"
          fill="none"
          aria-hidden="true"
          className="h-full w-full text-muted-foreground/70"
        >
          <circle cx="8" cy="8" r="6" stroke="currentColor" strokeWidth="2" />
        </svg>
      );
  }
}

/** Draws `indicator`; `data-run-state` carries the host's state, if any. */
export function EnvIndicator({
  indicator,
  runState,
  className,
}: {
  indicator: Indicator;
  runState?: RunState;
  className?: string;
}) {
  return (
    <span
      role="img"
      aria-label={indicator.title}
      title={indicator.title}
      data-indicator={indicator.kind}
      data-run-state={runState}
      data-dirty={indicator.dirty || undefined}
      className={cn("relative inline-flex h-3.5 w-3.5 shrink-0", className)}
    >
      <Glyph kind={indicator.kind} />
      {indicator.dirty && (
        <span className="absolute -right-0.5 -bottom-0.5 h-1.5 w-1.5 rounded-full bg-primary ring-1 ring-background" />
      )}
    </span>
  );
}

/** Whether `env`'s draft is in a revision, where the store keeps revisions
 *  (`undefined` otherwise, and while it is asked). */
export function useDraftState(env: EnvDoc | undefined): DraftState | undefined {
  const revisions = useWorkspace().revisions;
  const [state, setState] = useState<DraftState>();
  const id = env?.id;
  const head = env?.head;
  const selection = env?.selection;
  useEffect(() => {
    if (!id || !revisions) {
      setState(undefined);
      return;
    }
    if (head === undefined) {
      setState(selection ? "uncommitted" : undefined);
      return;
    }
    if (!selection) {
      setState("committed");
      return;
    }
    let cancelled = false;
    Promise.all([revisions.list(id), selectionDigest(selection)]).then(
      ([list, digest]) => {
        if (cancelled) return;
        const last = list[0];
        setState(last && last.digest !== digest ? "changed" : "committed");
      },
      () => !cancelled && setState(undefined),
    );
    return () => {
      cancelled = true;
    };
  }, [id, head, selection, revisions, revisions?.version]);
  return state;
}

/** `env`'s indicator, from its plan, the host's status and its draft. */
export function EnvStatusIndicator({
  env,
  plan,
  clashing,
  status,
  className,
}: {
  env: EnvDoc;
  plan: MemberPlan | undefined;
  clashing?: boolean;
  status?: EnvironmentStatus;
  className?: string;
}) {
  const draft = useDraftState(env);
  return (
    <EnvIndicator
      indicator={environmentIndicator({ plan, clashing, status, draft })}
      runState={status?.state}
      className={className}
    />
  );
}
