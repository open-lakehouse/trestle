import { cn } from "@open-lakehouse/ui-kit";
import type { NodeProps } from "@xyflow/react";
import type { GraphNodeDto } from "../../types";
import { roleStyle } from "../roleTheme";
import { BaseNode } from "./BaseNode";

/** The `data` payload a module node carries (set in `useTopologyLayout`). */
export interface ModuleNodeData {
  node: GraphNodeDto;
  [key: string]: unknown;
}

/**
 * One module in the environment "markitecture": a role-colored header
 * (icon + role) and the module's display name, plus its placement. Fixed size
 * (240×92) — MUST match SIZE in `useTopologyLayout` so edge handles anchor at
 * the box center.
 */
export function ModuleNode({ data, selected }: NodeProps) {
  const { node } = data as ModuleNodeData;
  const style = roleStyle(node.role);
  const Icon = style.icon;
  const label = node.display_name ?? node.id;
  const placement = node.placement ?? undefined;

  return (
    <BaseNode selected={selected} className="h-[92px] w-[240px]">
      <div
        className={cn(
          "flex items-center gap-2 px-3 pt-2 text-xs font-medium",
          style.accent,
        )}
      >
        <Icon className="h-3.5 w-3.5" />
        {(node.role ?? "service").toUpperCase().replace(/_/g, " ")}
      </div>
      <div className="px-3 pb-1 pt-0.5">
        <div
          className="truncate text-sm font-semibold text-card-foreground"
          title={label}
        >
          {label}
        </div>
        {placement && (
          <div
            className="truncate text-[11px] text-muted-foreground"
            title={placement}
          >
            {placement}
          </div>
        )}
      </div>
    </BaseNode>
  );
}
