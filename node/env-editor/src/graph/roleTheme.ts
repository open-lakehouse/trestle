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
  Server,
  ShieldCheck,
  Workflow,
} from "lucide-react";

export interface RoleStyle {
  icon: LucideIcon;
  /** The function a component with this role fills, as a headline
   *  (e.g. "Identity provider"); the implementation name is the detail. */
  label: string;
  /** Tailwind text-color utility for the icon + role label. */
  accent: string;
}

// Well-known roles from `olai_stack_topology::Role`. Unknown roles fall back to
// `DEFAULT_ROLE_STYLE`.
const ROLE_STYLES: Record<string, RoleStyle> = {
  gateway: { label: "API gateway", icon: DoorOpen, accent: "text-primary" },
  object_store: {
    label: "Object store",
    icon: HardDrive,
    accent: "text-[hsl(var(--status-ready))]",
  },
  relational_db: {
    label: "Relational database",
    icon: Database,
    accent: "text-[hsl(var(--status-in-progress))]",
  },
  data_catalog: {
    label: "Data catalog",
    icon: Boxes,
    accent: "text-[hsl(var(--status-done))]",
  },
  sql_engine: { label: "SQL engine", icon: Gauge, accent: "text-primary" },
  experiment_tracking: {
    label: "Experiment tracking",
    icon: Workflow,
    accent: "text-[hsl(var(--status-in-progress))]",
  },
  tracing: {
    label: "Tracing",
    icon: Network,
    accent: "text-[hsl(var(--status-ready))]",
  },
  lineage: {
    label: "Lineage",
    icon: Workflow,
    accent: "text-[hsl(var(--status-done))]",
  },
  auth: {
    label: "Identity provider",
    icon: ShieldCheck,
    accent: "text-[hsl(var(--status-blocked))]",
  },
};

export const DEFAULT_ROLE_STYLE: RoleStyle = {
  icon: Server,
  label: "Service",
  accent: "text-muted-foreground",
};

/** The style for a role string, or the neutral default when unknown/absent.
 *  An unknown role keeps the neutral look but is headlined by its own name. */
export function roleStyle(role: string | null | undefined): RoleStyle {
  if (!role) return DEFAULT_ROLE_STYLE;
  return (
    ROLE_STYLES[role] ?? {
      ...DEFAULT_ROLE_STYLE,
      label: role.replace(/_/g, " ").replace(/^\w/, (c) => c.toUpperCase()),
    }
  );
}
