// Role → icon + accent, for coloring the "markitecture" nodes. Uses only
// semantic Tailwind utilities and the shared status accents from the ui-kit
// DESIGN.md token contract (never hardcoded hex), so nodes re-theme with the host.

import {
  Boxes,
  Database,
  DoorOpen,
  Gauge,
  HardDrive,
  type LucideIcon,
  Network,
  ScrollText,
  Server,
  Workflow,
} from "lucide-react";

export interface RoleStyle {
  icon: LucideIcon;
  /** Tailwind text-color utility for the icon + role label. */
  accent: string;
}

// Well-known roles from `olai_stack_topology::Role`. Unknown roles fall back to
// `DEFAULT_ROLE_STYLE`.
const ROLE_STYLES: Record<string, RoleStyle> = {
  gateway: { icon: DoorOpen, accent: "text-primary" },
  object_store: { icon: HardDrive, accent: "text-[hsl(var(--status-ready))]" },
  relational_db: {
    icon: Database,
    accent: "text-[hsl(var(--status-in-progress))]",
  },
  data_catalog: { icon: Boxes, accent: "text-[hsl(var(--status-done))]" },
  sql_engine: { icon: Gauge, accent: "text-primary" },
  experiment_tracking: {
    icon: Workflow,
    accent: "text-[hsl(var(--status-in-progress))]",
  },
  tracing: { icon: Network, accent: "text-[hsl(var(--status-ready))]" },
  lineage: { icon: Workflow, accent: "text-[hsl(var(--status-done))]" },
  auth: { icon: ScrollText, accent: "text-[hsl(var(--status-blocked))]" },
};

export const DEFAULT_ROLE_STYLE: RoleStyle = {
  icon: Server,
  accent: "text-muted-foreground",
};

/** The style for a role string, or the neutral default when unknown/absent. */
export function roleStyle(role: string | null | undefined): RoleStyle {
  if (!role) return DEFAULT_ROLE_STYLE;
  return ROLE_STYLES[role] ?? DEFAULT_ROLE_STYLE;
}
