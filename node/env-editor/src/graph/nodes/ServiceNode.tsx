import { cn } from "@open-lakehouse/ui-kit";
import type { NodeProps } from "@xyflow/react";
import type { GraphNodeDto, ServiceNodeDto } from "../../types";
import { roleStyle } from "../roleTheme";
import { BaseNode } from "./BaseNode";

/** The `data` payload a service node carries (set in `useTopologyLayout`). */
export interface ServiceNodeData {
  service: ServiceNodeDto;
  /** The owning module (absent only for the host node). */
  module?: GraphNodeDto;
  /** Rendered inside its module's group, so the group header names the module. */
  inGroup: boolean;
  [key: string]: unknown;
}

/** `:9080 :9100` for published ports, else the in-network ones. */
function portsLabel(service: ServiceNodeDto): string | undefined {
  if (service.published.length > 0) {
    return service.published.map((p) => `:${p.host}→${p.container}`).join(" ");
  }
  if (service.exposed.length > 0) {
    return service.exposed.map((p) => `:${p}`).join(" ");
  }
  return undefined;
}

/**
 * One runtime service in the environment "markitecture": a role-colored header
 * (icon + role) and a title, plus its ports. A standalone service is titled by
 * its module (it *is* the module); inside a group, the group header names the
 * module, so the node is titled by its compose service name. An `external`
 * service (placed outside compose) gets a dashed border. Fixed size
 * (240×92) — MUST match SERVICE_SIZE in `useTopologyLayout`.
 */
export function ServiceNode({ data, selected }: NodeProps) {
  const { service, module, inGroup } = data as ServiceNodeData;
  // A sidecar has no declared role of its own; it borrows its module's accent.
  const role = service.role ?? module?.role;
  const style = roleStyle(role);
  const Icon = style.icon;
  const title = inGroup
    ? service.id
    : (module?.display_name ?? module?.id ?? service.id);
  const roleLabel = service.role
    ? service.role.toUpperCase().replace(/_/g, " ")
    : "SIDECAR";
  const subtitle = [inGroup ? undefined : service.id, portsLabel(service)]
    .filter(Boolean)
    .join(" · ");

  return (
    <BaseNode
      selected={selected}
      className={cn(
        "h-[92px] w-[240px]",
        service.kind === "external" && "border-dashed",
      )}
    >
      <div
        className={cn(
          "flex items-center gap-2 px-3 pt-2 text-xs font-medium",
          style.accent,
        )}
      >
        <Icon className="h-3.5 w-3.5" />
        {roleLabel}
        {service.kind === "external" && (
          <span className="ml-auto text-[10px] text-muted-foreground">
            EXTERNAL
          </span>
        )}
      </div>
      <div className="px-3 pb-1 pt-0.5">
        <div
          className="truncate text-sm font-semibold text-card-foreground"
          title={title}
        >
          {title}
        </div>
        {subtitle && (
          <div
            className="truncate font-mono text-[11px] text-muted-foreground"
            title={subtitle}
          >
            {subtitle}
          </div>
        )}
      </div>
    </BaseNode>
  );
}
