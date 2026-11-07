// How a host reports something it runs (an environment, one of its services)
// to the editor. The editor only draws these: what "running" means, and how
// the host found out, stay on the host's side.

/** Where a running thing is in its lifecycle, as the host sees it. */
export type RunState =
  | "stopped"
  | "starting"
  | "running"
  | "stopping"
  | "degraded"
  | "failed"
  | "unknown";

/** A service's state on the canvas (see `NodeSlots.status`). */
export interface NodeStatus {
  state: RunState;
  /** Short text for the state, e.g. "healthy" or "exited (1)"; defaults to
   *  the state's name. */
  label?: string;
  /** More detail, shown in the tooltip. */
  detail?: string;
}

/** The text for a status: its label, else the state's name. */
export function runStateLabel(status: { state: RunState; label?: string }) {
  return status.label ?? status.state;
}

/** A status as a tooltip: the label, then any detail. */
export function runStateTitle(status: {
  state: RunState;
  label?: string;
  detail?: string;
}) {
  const label = runStateLabel(status);
  return status.detail ? `${label}: ${status.detail}` : label;
}
