import { cn } from "@open-lakehouse/ui-kit";
import { Handle, Position } from "@xyflow/react";
import type { CSSProperties, ReactNode } from "react";
import {
  BOTTOM_IN,
  BOTTOM_OUT,
  EAST_IN,
  EAST_OUT,
  GUTTER_INSET,
  TOP_IN,
  TOP_OUT,
} from "../lateralEdges";
import { COMPONENT_IN, COMPONENT_OUT, NODE_HEADER } from "../useTopologyLayout";

/** A small, quiet connection dot: edges carry the meaning, not the handles. */
export const HANDLE_CLASS =
  "!h-1.5 !w-1.5 !min-h-0 !min-w-0 !border-0 !bg-muted-foreground/50";

const LATERAL_HANDLE_CLASS = cn(HANDLE_CLASS, "!opacity-0");

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
      {/* Where lateral (service-to-service) routes attach: top and bottom
          between neighbours, the east side through the column's gutter (see
          `routeLateral`). Invisible: most cards have no lateral edge. */}
      {[TOP_IN, TOP_OUT].map((id) => (
        <Handle
          key={id}
          id={id}
          type={id === TOP_IN ? "target" : "source"}
          position={Position.Top}
          className={LATERAL_HANDLE_CLASS}
        />
      ))}
      {[BOTTOM_IN, BOTTOM_OUT].map((id) => (
        <Handle
          key={id}
          id={id}
          type={id === BOTTOM_IN ? "target" : "source"}
          position={Position.Bottom}
          className={LATERAL_HANDLE_CLASS}
        />
      ))}
      {[EAST_IN, EAST_OUT].map((id) => (
        <Handle
          key={id}
          id={id}
          type={id === EAST_IN ? "target" : "source"}
          position={Position.Right}
          className={LATERAL_HANDLE_CLASS}
          style={{ top: GUTTER_INSET }}
        />
      ))}
    </div>
  );
}
