import { cn } from "@open-lakehouse/ui-kit";
import { Handle, type NodeProps, Position } from "@xyflow/react";
import { Lock } from "lucide-react";
import type { ReactNode } from "react";
import type { ExposedDto, GraphNodeDto, SurfaceDto } from "../../types";
import { ModuleIcon } from "../projectIcons";
import { roleStyle } from "../roleTheme";
import {
  type AccessCheck,
  type AccessCheckKind,
  checkHandle,
  GATEWAY_CHECK_ROW,
  GATEWAY_ENDPOINT_LINE,
  GATEWAY_ROW_PAD,
  GATEWAY_ROW_TITLE,
  GATEWAY_SECTION_HEADER,
  GATEWAY_WIDTH,
  type GatewaySection,
  gatewayRowHeight,
  routeHandle,
  surfaceHandle,
} from "../useTopologyLayout";
import { cardClass, HANDLE_CLASS } from "./BaseNode";
import { NodeActions } from "./NodeActions";
import { NodeHeader } from "./NodeHeader";

/** The `data` payload the gateway node carries (set in `useTopologyLayout`). */
export interface GatewayNodeData {
  node: GraphNodeDto;
  /** One section per surface (listener), each with one row per backend. */
  sections: GatewaySection[];
  /** The request checks it delegates, in order (sign-in, then policy). */
  checks: AccessCheck[];
  /** The route handles an edge leaves from; other rows draw no handle. */
  anchored: string[];
  /** Component id → display name, for row titles. */
  names: Record<string, string>;
  /** Component id → role, for row icons. */
  roles: Record<string, string | null | undefined>;
  [key: string]: unknown;
}

const KIND_LABEL: Record<ExposedDto["kind"], string> = {
  api: "API",
  ui: "UI",
  service: "SVC",
};

const CHECK_LABEL: Record<AccessCheckKind, { step: string; hint: string }> = {
  authenticates: {
    step: "Sign-in · forward auth",
    hint: "Gated requests are sent to the identity provider to sign the user in.",
  },
  authorizes: {
    step: "Authorization · policy check",
    hint: "Each signed-in request is then checked against the policy engine's rules.",
  },
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

/** A section divider + title. The divider lives inside the fixed-height
 *  header (border-box), so it doesn't shift the rows off their ELK ports. */
function SectionHeader({
  className,
  title,
  children,
}: {
  className: string;
  title?: string;
  children: ReactNode;
}) {
  return (
    <div
      className={cn(
        "relative flex items-center justify-between gap-2 border-t border-border px-3 text-[11px] font-semibold uppercase tracking-wide",
        className,
      )}
      style={{ height: GATEWAY_SECTION_HEADER }}
      title={title}
    >
      {children}
    </div>
  );
}

/**
 * The gateway as the environment's front door, split by surface: first the
 * Lakehouse API (the platform's APIs and UIs, stitched together under path
 * prefixes), then each service endpoint (e.g. an S3-compatible store) that
 * needs its own origin. Locally each surface is a host port; in a hosted
 * deployment, each would be its own subdomain. Clients enter each section
 * through its own handle, and each backend row — its endpoints one per line —
 * has its own outbound handle, with a lock on gated routes. Last comes access
 * control: the checks gated requests pass, each with the handle its edge
 * leaves from. Geometry MUST match the GATEWAY_* constants in
 * `useTopologyLayout` (ELK places the ports there).
 */
export function GatewayNode({ data, selected }: NodeProps) {
  const { node, sections, checks, anchored, names, roles } =
    data as GatewayNodeData;

  return (
    <div className={cardClass(selected)} style={{ width: GATEWAY_WIDTH }}>
      <NodeActions id={node.id} selected={selected} />
      <NodeHeader node={node} />
      {sections.map(({ surface, rows }) => (
        <div key={surface.host_port}>
          <SectionHeader
            className={
              surface.kind === "platform"
                ? "bg-primary/10 text-primary"
                : "bg-muted text-muted-foreground"
            }
            title={`${SURFACE_HINT[surface.kind]} Locally :${surface.host_port}; hosted, its own subdomain.`}
          >
            <span className="truncate">{surfaceLabel(surface)}</span>
            <span className="font-mono normal-case">:{surface.host_port}</span>
            <Handle
              id={surfaceHandle(surface.host_port)}
              type="target"
              position={Position.Left}
              className={HANDLE_CLASS}
            />
          </SectionHeader>
          {rows.map((row) => {
            const handle = routeHandle(surface.host_port, row.module);
            return (
              <div
                key={row.module}
                className="relative flex flex-col px-3"
                style={{
                  height: gatewayRowHeight(row),
                  paddingTop: GATEWAY_ROW_PAD,
                  paddingBottom: GATEWAY_ROW_PAD,
                }}
              >
                <div
                  className="flex items-center gap-1.5 text-xs font-medium"
                  style={{ height: GATEWAY_ROW_TITLE }}
                >
                  <ModuleIcon
                    moduleId={row.module}
                    role={roles[row.module]}
                    className="h-3 w-3 shrink-0 text-muted-foreground"
                  />
                  <span className="truncate">
                    {names[row.module] ?? row.module}
                  </span>
                </div>
                {/* One endpoint per line: the row lists what this backend exposes. */}
                {row.exposes.map((e) => (
                  <div
                    key={`${e.endpoint}:${e.prefix}`}
                    className="flex items-center gap-1 truncate pl-[18px] font-mono text-[10px] text-muted-foreground"
                    style={{ height: GATEWAY_ENDPOINT_LINE }}
                    title={`${KIND_LABEL[e.kind]} ${e.prefix}${e.gated ? " (requires sign-in)" : ""}`}
                  >
                    <span className="w-7 shrink-0 rounded bg-muted text-center text-[9px] font-semibold">
                      {KIND_LABEL[e.kind]}
                    </span>
                    <span className="truncate">{e.prefix}</span>
                    {e.gated && <Lock className="h-2.5 w-2.5 shrink-0" />}
                  </div>
                ))}
                {anchored.includes(handle) && (
                  <Handle
                    id={handle}
                    type="source"
                    position={Position.Right}
                    className={HANDLE_CLASS}
                  />
                )}
              </div>
            );
          })}
        </div>
      ))}
      {checks.length > 0 && (
        <div>
          <SectionHeader
            className="bg-muted text-muted-foreground"
            title="The checks a gated request passes, in order, before it reaches a backend."
          >
            <span className="truncate">Access control</span>
            <Lock className="h-3 w-3 shrink-0" />
          </SectionHeader>
          {checks.map((check) => {
            const label = CHECK_LABEL[check.kind];
            const role = roles[check.module];
            return (
              <div
                key={check.kind}
                className="relative flex items-center gap-2.5 px-3"
                style={{ height: GATEWAY_CHECK_ROW }}
                title={label.hint}
              >
                <div
                  className={cn(
                    "flex h-6 w-6 shrink-0 items-center justify-center rounded bg-current/10",
                    roleStyle(role).accent,
                  )}
                >
                  <ModuleIcon
                    moduleId={check.module}
                    role={role}
                    className="h-3.5 w-3.5"
                  />
                </div>
                <div className="min-w-0">
                  <div className="truncate text-xs font-medium leading-4">
                    {names[check.module] ?? check.module}
                  </div>
                  <div className="truncate text-[10px] leading-4 text-muted-foreground">
                    {label.step}
                  </div>
                </div>
                <Handle
                  id={checkHandle(check.kind)}
                  type="source"
                  position={Position.Right}
                  className={HANDLE_CLASS}
                />
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
