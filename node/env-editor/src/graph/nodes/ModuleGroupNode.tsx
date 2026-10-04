import { cn } from "@open-lakehouse/ui-kit";
import type { NodeProps } from "@xyflow/react";
import type { GraphNodeDto } from "../../types";
import { roleStyle } from "../roleTheme";

/** The `data` payload a module group carries (set in `useTopologyLayout`). */
export interface ModuleGroupNodeData {
  module: GraphNodeDto;
  [key: string]: unknown;
}

/**
 * A module that runs several long-running services (e.g. rustfs: rustfs +
 * sts-shim), drawn as a frame around them. Its size comes from ELK; the header
 * band's height MUST match GROUP_HEADER in `useTopologyLayout`.
 */
export function ModuleGroupNode({ data, selected }: NodeProps) {
  const { module } = data as ModuleGroupNodeData;
  const style = roleStyle(module.role);
  const Icon = style.icon;
  const label = module.display_name ?? module.id;

  return (
    <div
      className={cn(
        "h-full w-full cursor-pointer rounded-xl border bg-muted/40",
        selected ? "border-primary ring-2 ring-primary/30" : "border-border",
      )}
    >
      <div
        className={cn(
          "flex h-[36px] items-center gap-2 px-3 text-xs font-medium",
          style.accent,
        )}
        title={module.summary ?? label}
      >
        <Icon className="h-3.5 w-3.5" />
        <span className="truncate text-sm font-semibold text-card-foreground">
          {label}
        </span>
      </div>
    </div>
  );
}
