import { cn } from "@open-lakehouse/ui-kit";
import { Handle, Position } from "@xyflow/react";
import type { ReactNode } from "react";

export interface BaseNodeProps {
  selected?: boolean;
  className?: string;
  children: ReactNode;
}

/**
 * Shared chrome for every module node: a bordered card with left (target) and
 * right (source) handles for the LEFT→RIGHT dependency layout. Node components
 * supply only their content; click handling lives on the ReactFlow canvas
 * (`onNodeClick`). Adapted from the headwaters lineage `BaseNode`.
 */
export function BaseNode({ selected, className, children }: BaseNodeProps) {
  return (
    <div
      className={cn(
        // flex-col + justify-center vertically centers content within the fixed
        // node height, so same-row node centers line up and edges stay straight
        // (the box size must match SIZE in useTopologyLayout).
        "flex cursor-pointer flex-col justify-center overflow-hidden rounded-lg border bg-card text-card-foreground shadow-sm transition-shadow hover:shadow-md",
        selected ? "border-primary ring-2 ring-primary/30" : "border-border",
        className,
      )}
    >
      <Handle
        type="target"
        position={Position.Left}
        className="!border-border !bg-muted-foreground"
      />
      {children}
      <Handle
        type="source"
        position={Position.Right}
        className="!border-border !bg-muted-foreground"
      />
    </div>
  );
}
