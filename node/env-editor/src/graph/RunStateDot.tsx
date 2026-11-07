// The dot that draws a host-reported run state (see `runState.ts`).

import { cn } from "@open-lakehouse/ui-kit";
import { type RunState, runStateTitle } from "./runState.js";

const DOT: Record<RunState, string> = {
  running: "bg-success",
  starting: "bg-status-ready animate-pulse",
  stopping: "bg-status-ready animate-pulse",
  degraded: "bg-status-ready",
  failed: "bg-destructive",
  stopped: "bg-muted-foreground/50",
  unknown: "border border-muted-foreground/60 bg-transparent",
};

/** A small colored dot for a run state. */
export function RunStateDot({
  status,
  className,
}: {
  status: { state: RunState; label?: string; detail?: string };
  className?: string;
}) {
  const title = runStateTitle(status);
  return (
    <span
      role="img"
      aria-label={title}
      title={title}
      data-run-state={status.state}
      className={cn(
        "inline-block h-2 w-2 shrink-0 rounded-full",
        DOT[status.state],
        className,
      )}
    />
  );
}
