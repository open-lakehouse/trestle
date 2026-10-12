import { cn } from "@open-lakehouse/ui-kit";
import { Handle, Position } from "@xyflow/react";
import type { CSSProperties, ReactNode } from "react";
import { COMPONENT_IN, COMPONENT_OUT, NODE_HEADER } from "../useTopologyLayout";

/** A small, quiet connection dot: edges carry the meaning, not the handles. */
export const HANDLE_CLASS =
  "!h-1.5 !w-1.5 !min-h-0 !min-w-0 !border-0 !bg-muted-foreground/50";

/** The card surface every component and gateway node shares. */
export function cardClass(selected: boolean | undefined) {
  return cn(
    "cursor-pointer overflow-hidden rounded-lg border bg-card text-card-foreground shadow-sm transition-shadow hover:shadow-md",
    selected ? "border-primary ring-2 ring-primary/30" : "border-border",
  );
}

export interface BaseNodeProps {
  selected?: boolean;
  className?: string;
  style?: CSSProperties;
  children: ReactNode;
}

export function BaseNode({
  selected,
  className,
  style,
  children,
}: BaseNodeProps) {
  return (
    <div
      className={cn("relative flex flex-col", cardClass(selected), className)}
      style={style}
    >
      {/* Level with the header: the ports ELK places (see `componentPorts`). */}
      <Handle
        id={COMPONENT_IN}
        type="target"
        position={Position.Left}
        className={HANDLE_CLASS}
        style={{ top: NODE_HEADER / 2 }}
      />
      {children}
      <Handle
        id={COMPONENT_OUT}
        type="source"
        position={Position.Right}
        className={HANDLE_CLASS}
        style={{ top: NODE_HEADER / 2 }}
      />
    </div>
  );
}
