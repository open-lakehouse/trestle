import { cn } from "@open-lakehouse/ui-kit";
import { Handle, type NodeProps, Position } from "@xyflow/react";
import { Box, Database, Plus, X } from "lucide-react";
import type { ReactNode } from "react";
import type { GraphNodeDto, ProvisionedDto } from "../../types";
import { useCanvasContext } from "../canvasActions";
import {
  COMPONENT_WIDTH,
  componentFacts,
  componentHeight,
  FACT_LINE,
  factsHeight,
  provisionLabel,
  RESOURCE_ROW,
  RESOURCE_SECTION_HEADER,
  type ResourceSection,
  resourceHandle,
  resourceSections,
} from "../useTopologyLayout";
import { BaseNode, HANDLE_CLASS } from "./BaseNode";
import { NodeActions } from "./NodeActions";
import { NodeHeader } from "./NodeHeader";

export interface ComponentNodeData {
  node: GraphNodeDto;
  /** Component id → display name, for naming a resource's consumers. */
  names: Record<string, string>;
  [key: string]: unknown;
}

/**
 * A functional component: the shared header, a labeled line per fact (what it
 * offers), then for a provider one section per resource kind with a row per
 * database / bucket — each row with its own inbound handle, where the edges
 * of the components using it land. Its size is `componentHeight` ×
 * COMPONENT_WIDTH, the box ELK lays out, and the rows sit at the offsets
 * `componentPorts` places their ports.
 */
export function ComponentNode({ data, selected }: NodeProps) {
  const { node, names } = data as ComponentNodeData;
  const facts = componentFacts(node);

  return (
    <BaseNode
      selected={selected}
      className={cn(node.kind === "external" && "border-dashed")}
      style={{ width: COMPONENT_WIDTH, height: componentHeight(node) }}
    >
      <NodeActions id={node.id} selected={selected} />
      <NodeHeader node={node} />
      {facts.length > 0 && (
        <div
          className="flex shrink-0 flex-col justify-center border-t border-border px-3"
          style={{ height: factsHeight(node) }}
        >
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
      {resourceSections(node).map((section) => (
        <ResourceList
          key={section.kind}
          providerId={node.id}
          label={provisionLabel(node, section.kind)}
          section={section}
          names={names}
        />
      ))}
    </BaseNode>
  );
}

function ResourceList({
  providerId,
  label,
  section,
  names,
}: {
  providerId: string;
  label: string;
  section: ResourceSection;
  names: Record<string, string>;
}) {
  const actions = useCanvasContext()?.actions;
  const Icon = section.kind === "relational_db" ? Database : Box;
  return (
    <div className="shrink-0">
      <div
        className="flex items-center justify-between gap-2 border-t border-border bg-muted/60 px-3 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground"
        style={{ height: RESOURCE_SECTION_HEADER }}
      >
        <span>
          {label} · {section.items.length}
        </span>
        {actions && (
          <button
            type="button"
            className="nodrag -mr-1 flex h-5 w-5 items-center justify-center rounded hover:bg-background hover:text-foreground"
            aria-label={`Add ${label.toLowerCase()}`}
            title={`Add ${label.toLowerCase()}`}
            onClick={(e) => {
              e.stopPropagation();
              actions.addResource(providerId);
            }}
          >
            <Plus className="h-3 w-3" />
          </button>
        )}
      </div>
      {section.items.map((item) => (
        <ResourceRow
          key={item.name}
          item={item}
          icon={<Icon className="h-3 w-3 shrink-0 text-muted-foreground" />}
          names={names}
          onRemove={
            actions && item.consumers.length === 0
              ? () => actions.removeResource(item.resource, item.name)
              : undefined
          }
        />
      ))}
    </div>
  );
}

function ResourceRow({
  item,
  icon,
  names,
  onRemove,
}: {
  item: ProvisionedDto;
  icon: ReactNode;
  names: Record<string, string>;
  onRemove?: () => void;
}) {
  const users = item.consumers.map((c) => names[c] ?? c).join(", ");
  return (
    <div
      className="group relative flex items-center gap-1.5 px-3 text-[11px]"
      style={{ height: RESOURCE_ROW }}
      title={users ? `Used by ${users}` : "Added by you; no component uses it"}
    >
      <Handle
        id={resourceHandle(item.resource, item.name)}
        type="target"
        position={Position.Left}
        className={HANDLE_CLASS}
      />
      {icon}
      <span className="min-w-0 flex-1 truncate font-mono font-medium">
        {item.name}
      </span>
      <span className="max-w-[45%] shrink-0 truncate text-[10px] text-muted-foreground">
        {users || "extra"}
      </span>
      {onRemove && (
        <button
          type="button"
          className="nodrag -mr-1 hidden h-4 w-4 items-center justify-center rounded text-muted-foreground hover:text-destructive group-hover:flex"
          aria-label={`Remove ${item.name}`}
          onClick={(e) => {
            e.stopPropagation();
            onRemove();
          }}
        >
          <X className="h-3 w-3" />
        </button>
      )}
    </div>
  );
}
