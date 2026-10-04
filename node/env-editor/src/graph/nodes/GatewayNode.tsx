import { cn } from "@open-lakehouse/ui-kit";
import { Handle, type NodeProps, Position } from "@xyflow/react";
import { Lock } from "lucide-react";
import type { ExposedDto, GraphNodeDto } from "../../types";
import { roleStyle } from "../roleTheme";
import {
  GATEWAY_HEADER,
  GATEWAY_ROW,
  GATEWAY_WIDTH,
  type GatewayRow,
  routeHandle,
} from "../useTopologyLayout";

/** The `data` payload the gateway node carries (set in `useTopologyLayout`). */
export interface GatewayNodeData {
  node: GraphNodeDto;
  /** The exposed surface grouped by backend, one row per backend. */
  rows: GatewayRow[];
  /** Backend id → display name, for row titles. */
  names: Record<string, string>;
  [key: string]: unknown;
}

const KIND_LABEL: Record<ExposedDto["kind"], string> = {
  api: "API",
  ui: "UI",
  service: "SVC",
};

/**
 * The gateway as the environment's single front door: it lists the surface it
 * exposes — API prefixes, UIs, whole-service listeners — one row per backend
 * it stitches in, with a lock on gated routes. Each row has its own handle, so
 * the route to that backend leaves from the row. Width and row geometry MUST
 * match GATEWAY_* in `useTopologyLayout` (ELK places the row ports there).
 */
export function GatewayNode({ data, selected }: NodeProps) {
  const { node, rows, names } = data as GatewayNodeData;
  const style = roleStyle(node.role);
  const Icon = style.icon;
  const ports = [
    ...new Set(rows.flatMap((r) => r.exposes.map((e) => e.host_port))),
  ];

  return (
    <div
      className={cn(
        "relative cursor-pointer overflow-hidden rounded-lg border bg-card text-card-foreground shadow-sm transition-shadow hover:shadow-md",
        selected ? "border-primary ring-2 ring-primary/30" : "border-border",
      )}
      style={{ width: GATEWAY_WIDTH }}
    >
      <Handle
        type="target"
        position={Position.Left}
        className="!border-border !bg-muted-foreground"
      />
      <div
        className="flex flex-col justify-center border-b border-border px-3"
        style={{ height: GATEWAY_HEADER }}
      >
        <div
          className={cn(
            "flex items-center gap-2 text-xs font-medium",
            style.accent,
          )}
        >
          <Icon className="h-3.5 w-3.5" />
          {style.label.toUpperCase()}
        </div>
        <div className="flex items-baseline justify-between gap-2">
          <span className="truncate text-sm font-semibold">
            {node.display_name ?? node.id}
          </span>
          <span className="font-mono text-[11px] text-muted-foreground">
            {ports.map((p) => `:${p}`).join(" ")}
          </span>
        </div>
      </div>
      {rows.map((row) => (
        <div
          key={row.module}
          className="relative flex flex-col justify-center px-3"
          style={{ height: GATEWAY_ROW }}
        >
          <div className="truncate text-xs font-medium">
            {names[row.module] ?? row.module}
          </div>
          <div className="flex items-center gap-1 truncate font-mono text-[10px] text-muted-foreground">
            {row.exposes.map((e) => (
              <span
                key={`${e.endpoint}:${e.prefix}`}
                className="flex items-center gap-0.5"
                title={`${KIND_LABEL[e.kind]} ${e.prefix} on :${e.host_port}${e.gated ? " (requires sign-in)" : ""}`}
              >
                <span className="rounded bg-muted px-1 text-[9px] font-semibold">
                  {KIND_LABEL[e.kind]}
                </span>
                {e.kind === "service" ? `:${e.host_port}` : e.prefix}
                {e.gated && <Lock className="h-2.5 w-2.5" />}
              </span>
            ))}
          </div>
          <Handle
            id={routeHandle(row.module)}
            type="source"
            position={Position.Right}
            className="!border-border !bg-muted-foreground"
          />
        </div>
      ))}
    </div>
  );
}
