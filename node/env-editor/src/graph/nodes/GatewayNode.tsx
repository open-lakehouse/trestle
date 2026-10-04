import { cn } from "@open-lakehouse/ui-kit";
import { Handle, type NodeProps, Position } from "@xyflow/react";
import { Lock } from "lucide-react";
import type { ExposedDto, GraphNodeDto, SurfaceDto } from "../../types";
import { roleStyle } from "../roleTheme";
import {
  GATEWAY_HEADER,
  GATEWAY_ROW,
  GATEWAY_SECTION_HEADER,
  GATEWAY_WIDTH,
  type GatewaySection,
  routeHandle,
  surfaceHandle,
} from "../useTopologyLayout";

/** The `data` payload the gateway node carries (set in `useTopologyLayout`). */
export interface GatewayNodeData {
  node: GraphNodeDto;
  /** One section per surface (listener), each with one row per backend. */
  sections: GatewaySection[];
  /** Backend id → display name, for row titles. */
  names: Record<string, string>;
  [key: string]: unknown;
}

const KIND_LABEL: Record<ExposedDto["kind"], string> = {
  api: "API",
  ui: "UI",
  service: "SVC",
};

/** A surface's headline: the platform's own API, or the protocol a service
 *  endpoint speaks. */
function surfaceLabel(surface: SurfaceDto): string {
  if (surface.kind === "platform") return "Lakehouse API";
  const protocols = surface.protocols.join(", ");
  return protocols ? `${protocols} endpoint` : "Service endpoint";
}

const SURFACE_HINT: Record<SurfaceDto["kind"], string> = {
  platform:
    "The platform surface: component APIs and UIs stitched together under path prefixes.",
  service:
    "A service endpoint clients address at its own origin in its wire protocol, so it can't share the platform's prefixes.",
};

/**
 * The gateway as the environment's front door, split by surface: first the
 * Lakehouse API (the platform's APIs and UIs, stitched together under path
 * prefixes), then each service endpoint (e.g. an S3-compatible store) that
 * needs its own origin. Locally each surface is a host port; in a hosted
 * deployment, each would be its own subdomain. Clients enter each section
 * through its own handle, and each backend row has its own outbound handle,
 * with a lock on gated routes. Geometry MUST match GATEWAY_* in
 * `useTopologyLayout` (ELK places the ports there).
 */
export function GatewayNode({ data, selected }: NodeProps) {
  const { node, sections, names } = data as GatewayNodeData;
  const style = roleStyle(node.role);
  const Icon = style.icon;

  return (
    <div
      className={cn(
        "cursor-pointer overflow-hidden rounded-lg border bg-card text-card-foreground shadow-sm transition-shadow hover:shadow-md",
        selected ? "border-primary ring-2 ring-primary/30" : "border-border",
      )}
      style={{ width: GATEWAY_WIDTH }}
    >
      <div
        className="flex flex-col justify-center px-3"
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
        <div className="truncate text-sm font-semibold">
          {node.display_name ?? node.id}
        </div>
      </div>
      {sections.map(({ surface, rows }) => (
        <div key={surface.host_port}>
          {/* The divider lives inside the fixed-height header (border-box), so
              it doesn't shift the rows off their ELK ports. */}
          <div
            className={cn(
              "relative flex items-center justify-between gap-2 border-t border-border px-3 text-[11px] font-semibold uppercase tracking-wide",
              surface.kind === "platform"
                ? "bg-primary/10 text-primary"
                : "bg-muted text-muted-foreground",
            )}
            style={{ height: GATEWAY_SECTION_HEADER }}
            title={`${SURFACE_HINT[surface.kind]} Locally :${surface.host_port}; hosted, its own subdomain.`}
          >
            <span className="truncate">{surfaceLabel(surface)}</span>
            <span className="font-mono normal-case">:{surface.host_port}</span>
            <Handle
              id={surfaceHandle(surface.host_port)}
              type="target"
              position={Position.Left}
              className="!border-border !bg-muted-foreground"
            />
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
                    title={`${KIND_LABEL[e.kind]} ${e.prefix}${e.gated ? " (requires sign-in)" : ""}`}
                  >
                    <span className="rounded bg-muted px-1 text-[9px] font-semibold">
                      {KIND_LABEL[e.kind]}
                    </span>
                    {e.prefix}
                    {e.gated && <Lock className="h-2.5 w-2.5" />}
                  </span>
                ))}
              </div>
              <Handle
                id={routeHandle(surface.host_port, row.module)}
                type="source"
                position={Position.Right}
                className="!border-border !bg-muted-foreground"
              />
            </div>
          ))}
        </div>
      ))}
    </div>
  );
}
