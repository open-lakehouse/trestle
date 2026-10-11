import { cn } from "@open-lakehouse/ui-kit";
import type { NodeProps } from "@xyflow/react";
import type { GraphNodeDto } from "../../types";
import {
  COMPONENT_WIDTH,
  componentFacts,
  componentHeight,
  FACT_LINE,
} from "../useTopologyLayout";
import { BaseNode } from "./BaseNode";
import { NodeHeader } from "./NodeHeader";

export interface ComponentNodeData {
  node: GraphNodeDto;
  [key: string]: unknown;
}

/**
 * A functional component: the shared header, then a labeled line per fact
 * (what it offers, what it provisions). Its size is `componentHeight` ×
 * COMPONENT_WIDTH, the box ELK lays out; the body centers its lines, so the
 * card border never pushes them out of that box.
 */
export function ComponentNode({ data, selected }: NodeProps) {
  const { node } = data as ComponentNodeData;
  const facts = componentFacts(node);

  return (
    <BaseNode
      selected={selected}
      className={cn(node.kind === "external" && "border-dashed")}
      style={{ width: COMPONENT_WIDTH, height: componentHeight(node) }}
    >
      <NodeHeader node={node} />
      {facts.length > 0 && (
        <div className="flex min-h-0 flex-1 flex-col justify-center border-t border-border px-3">
          {facts.map((f) => (
            <div
              key={f.label}
              className="flex items-center gap-2 text-[11px]"
              style={{ height: FACT_LINE }}
            >
              <span className="w-[68px] shrink-0 text-muted-foreground">
                {f.label}
              </span>
              <span className="truncate font-medium" title={f.value}>
                {f.value}
              </span>
            </div>
          ))}
        </div>
      )}
    </BaseNode>
  );
}
