import { cn } from "@open-lakehouse/ui-kit";
import type { NodeProps } from "@xyflow/react";
import type { GraphNodeDto } from "../../types";
import { roleStyle } from "../roleTheme";
import { BaseNode } from "./BaseNode";

/** The `data` payload a component node carries (set in `useTopologyLayout`). */
export interface ComponentNodeData {
  node: GraphNodeDto;
  [key: string]: unknown;
}

/**
 * One functional component: headlined by the function it fills (e.g.
 * "Object store", "Identity provider"), with the implementation as the title
 * and what it offers / provisions underneath. Helper containers are part of
 * the component, so they never get a node. An `external` component (run
 * outside the stack) gets a dashed border. Fixed size (240×92) — MUST match
 * COMPONENT_SIZE in `useTopologyLayout`.
 */
export function ComponentNode({ data, selected }: NodeProps) {
  const { node } = data as ComponentNodeData;
  const style = roleStyle(node.role);
  const Icon = style.icon;
  const title = node.display_name ?? node.id;
  const detail = [
    node.offers.join(", "),
    node.provisions.map((p) => p.name).join(", "),
  ]
    .filter(Boolean)
    .join(" · ");

  return (
    <BaseNode
      selected={selected}
      className={cn(
        "h-[92px] w-[240px]",
        node.kind === "external" && "border-dashed",
      )}
    >
      <div
        className={cn(
          "flex items-center gap-2 px-3 pt-2 text-xs font-medium",
          style.accent,
        )}
      >
        <Icon className="h-3.5 w-3.5" />
        {style.label.toUpperCase()}
        {node.kind === "external" && (
          <span className="ml-auto text-[10px] text-muted-foreground">
            EXTERNAL
          </span>
        )}
      </div>
      <div className="px-3 pb-1 pt-0.5">
        <div
          className="truncate text-sm font-semibold text-card-foreground"
          title={node.summary ?? title}
        >
          {title}
        </div>
        {detail && (
          <div
            className="truncate text-[11px] text-muted-foreground"
            title={detail}
          >
            {detail}
          </div>
        )}
      </div>
    </BaseNode>
  );
}
