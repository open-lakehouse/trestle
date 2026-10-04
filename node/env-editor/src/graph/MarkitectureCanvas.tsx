import {
  Background,
  type ColorMode,
  Controls,
  type Edge,
  MarkerType,
  type Node,
  type NodeTypes,
  Panel,
  ReactFlow,
  ReactFlowProvider,
  useReactFlow,
} from "@xyflow/react";
import {
  type CSSProperties,
  useCallback,
  useEffect,
  useMemo,
  useState,
} from "react";
import type {
  GraphDto,
  GraphNodeDto,
  TopologyEdgeDto,
  TopologyEdgeKind,
} from "../types";
import { HostNode } from "./nodes/HostNode";
import {
  ModuleGroupNode,
  type ModuleGroupNodeData,
} from "./nodes/ModuleGroupNode";
import { ServiceNode, type ServiceNodeData } from "./nodes/ServiceNode";
import { type TopologyEdgeData, useTopologyLayout } from "./useTopologyLayout";

const nodeTypes: NodeTypes = {
  service: ServiceNode,
  host: HostNode,
  moduleGroup: ModuleGroupNode,
};

export interface MarkitectureCanvasProps {
  /** The plan's runtime topology. */
  graph: GraphDto | undefined;
  /** Currently-selected module id, highlighted in the canvas. */
  selectedId?: string;
  /** Called when a module (a service node or a module group) is clicked. */
  onSelect?: (node: GraphNodeDto) => void;
  /**
   * Drives ReactFlow's built-in canvas/Controls/Background colors. Defaults to
   * `"system"`; a host with an explicit light/dark toggle can pass its active
   * theme so the canvas tracks the rest of the app.
   */
  colorMode?: ColorMode;
}

/**
 * Per-kind edge look. Strokes reference the raw HSL-component tokens (authored
 * on the host's :root/.dark), which are theme-reactive.
 */
const EDGE_STYLE: Record<
  TopologyEdgeKind,
  { label: string; stroke: string; dash?: string; animated: boolean }
> = {
  ingress: {
    label: "Host ports",
    stroke: "hsl(var(--muted-foreground))",
    animated: true,
  },
  route: {
    label: "Gateway route",
    stroke: "hsl(var(--primary))",
    animated: true,
  },
  authz: {
    label: "Auth check",
    stroke: "hsl(var(--status-blocked))",
    animated: true,
  },
  emulated: {
    label: "Emulated cloud host (TLS)",
    stroke: "hsl(var(--status-ready))",
    dash: "6 4",
    animated: false,
  },
  startup: {
    label: "Waits on (startup)",
    stroke: "hsl(var(--muted-foreground))",
    dash: "2 4",
    animated: false,
  },
};

const CONDITION_LABEL: Record<string, string> = {
  service_started: "started",
  service_healthy: "healthy",
  service_completed_successfully: "completed",
};

/** A short label for one typed edge. */
function edgeLabel(e: TopologyEdgeDto): string | undefined {
  switch (e.kind) {
    case "ingress":
      return e.host_ports.map((p) => `:${p}`).join(" ");
    case "route": {
      const [first, ...rest] = e.routes;
      const gated = e.routes.some((r) => r.gated) ? " · auth" : "";
      return `${first?.prefix ?? ""}${rest.length ? ` +${rest.length}` : ""}${gated}`;
    }
    case "authz":
      return "ext_authz";
    case "emulated":
      return `TLS :${e.port}`;
    case "startup":
      return undefined;
  }
}

/** The full detail of one typed edge, for the edge's accessible label. */
function edgeTitle(e: TopologyEdgeDto): string {
  switch (e.kind) {
    case "ingress":
      return `Published on host: ${e.host_ports.join(", ")}`;
    case "route":
      return e.routes
        .map(
          (r) =>
            `${r.prefix} (:${r.host_port})${r.rewrite ? ` → ${r.rewrite}` : ""}${r.gated ? " [auth]" : ""}`,
        )
        .join("\n");
    case "authz":
      return `Forward-auth check; login portal at ${e.portal_prefix}`;
    case "emulated":
      return `TLS :${e.port}\n${e.hosts.join("\n")}`;
    case "startup":
      return `Starts after ${e.to} is ${CONDITION_LABEL[e.condition] ?? e.condition}`;
  }
}

/**
 * Render a planned environment's runtime topology as a left-to-right
 * "markitecture": host → gateway → apps → backing stores. Purely
 * presentational: the app feeds it a live plan graph and Storybook feeds it a
 * fixture — same component, no planner coupling.
 */
export function MarkitectureCanvas(props: MarkitectureCanvasProps) {
  // useReactFlow requires a provider ancestor, so the flow lives inside one.
  return (
    <ReactFlowProvider>
      <MarkitectureFlow {...props} />
    </ReactFlowProvider>
  );
}

/** The module a node stands for (a service's owner, or a group's module). */
function nodeModule(node: Node): GraphNodeDto | undefined {
  return (node.data as Partial<ServiceNodeData & ModuleGroupNodeData>).module;
}

function MarkitectureFlow({
  graph,
  selectedId,
  onSelect,
  colorMode = "system",
}: MarkitectureCanvasProps) {
  const [showStartup, setShowStartup] = useState(true);
  const { nodes, edges } = useTopologyLayout(graph, showStartup);
  const { fitView } = useReactFlow();

  // Selection is by module: highlight its group and every service it runs.
  const decoratedNodes = useMemo(
    () =>
      nodes.map((n) => ({
        ...n,
        selected: !!selectedId && nodeModule(n)?.id === selectedId,
      })),
    [nodes, selectedId],
  );
  const selectedServices = useMemo(
    () =>
      new Set(
        decoratedNodes
          .filter((n) => n.selected && n.type !== "moduleGroup")
          .map((n) => n.id),
      ),
    [decoratedNodes],
  );

  // Each kind has its own stroke/dash; selecting a module emphasizes edges
  // incident to its services and dims the rest.
  const decoratedEdges = useMemo<Edge[]>(
    () =>
      edges.map((e) => {
        const data = e.data as TopologyEdgeData;
        const look = EDGE_STYLE[data.kind];
        const incident =
          selectedServices.has(e.source) || selectedServices.has(e.target);
        const dimmed = selectedServices.size > 0 && !incident;
        const label = [
          ...data.edges.map(edgeLabel),
          ...data.back.map((b) => {
            const l = edgeLabel(b);
            return l ? `↩ ${l}` : undefined;
          }),
        ]
          .filter((l): l is string => !!l)
          .join(" · ");
        const marker = (color: string) => ({
          type: MarkerType.ArrowClosed,
          width: 14,
          height: 14,
          color,
        });
        const style: CSSProperties = {
          stroke: look.stroke,
          strokeWidth: incident ? 2.5 : data.kind === "startup" ? 1 : 1.5,
          strokeDasharray: look.dash,
          opacity: dimmed ? 0.25 : incident ? 1 : 0.85,
          transition: "opacity 150ms ease",
        };
        return {
          ...e,
          animated: look.animated && !look.dash,
          label: label || undefined,
          ariaLabel: [...data.edges, ...data.back].map(edgeTitle).join("\n"),
          labelStyle: { fontSize: 10, fill: "hsl(var(--muted-foreground))" },
          labelBgStyle: { fill: "hsl(var(--background))", fillOpacity: 0.85 },
          labelBgPadding: [4, 2] as [number, number],
          markerEnd: marker(look.stroke),
          // A folded back-edge (client → gateway) gets the reverse arrowhead in
          // its own kind's color.
          markerStart:
            data.back.length > 0
              ? marker(EDGE_STYLE[data.back[0].kind].stroke)
              : undefined,
          style,
        };
      }),
    [edges, selectedServices],
  );

  // ELK resolves asynchronously, so nodes arrive after ReactFlow's initial mount
  // (when the `fitView` prop fits an empty graph). Refit imperatively whenever
  // the laid-out node set changes, keyed on node ids so it fires on real layout
  // changes, not selection-only re-renders.
  const nodeKey = nodes.map((n) => n.id).join("|");
  useEffect(() => {
    if (nodeKey === "") return;
    const raf = requestAnimationFrame(() => {
      void fitView({ padding: 0.15, duration: 200 });
    });
    return () => cancelAnimationFrame(raf);
  }, [nodeKey, fitView]);

  const handleNodeClick = useCallback(
    (_: unknown, node: Node) => {
      const module = nodeModule(node);
      if (module && onSelect) onSelect(module);
    },
    [onSelect],
  );

  const kinds = useMemo(
    () => new Set(graph?.edges.map((e) => e.kind) ?? []),
    [graph],
  );

  return (
    <ReactFlow
      nodes={decoratedNodes}
      edges={decoratedEdges}
      nodeTypes={nodeTypes}
      onNodeClick={handleNodeClick}
      colorMode={colorMode}
      fitView
      proOptions={{ hideAttribution: true }}
      minZoom={0.1}
    >
      <Background />
      <Controls showInteractive={false} />
      <Panel position="top-right">
        <Legend
          kinds={kinds}
          showStartup={showStartup}
          onToggleStartup={() => setShowStartup((v) => !v)}
        />
      </Panel>
    </ReactFlow>
  );
}

/** A compact key for the edge kinds present in the graph. */
function Legend({
  kinds,
  showStartup,
  onToggleStartup,
}: {
  kinds: Set<TopologyEdgeKind>;
  showStartup: boolean;
  onToggleStartup: () => void;
}) {
  const order: TopologyEdgeKind[] = [
    "ingress",
    "route",
    "authz",
    "emulated",
    "startup",
  ];
  return (
    <div className="rounded-md border border-border bg-card/90 px-3 py-2 text-[11px] text-muted-foreground shadow-sm">
      {order
        .filter((k) => kinds.has(k))
        .map((k) => {
          const look = EDGE_STYLE[k];
          return (
            <div key={k} className="flex items-center gap-2 py-0.5">
              <svg width="28" height="8" aria-hidden="true">
                <line
                  x1="0"
                  y1="4"
                  x2="28"
                  y2="4"
                  stroke={look.stroke}
                  strokeWidth="2"
                  // ReactFlow draws an animated edge as moving 5px dashes.
                  strokeDasharray={
                    look.dash ?? (look.animated ? "5" : undefined)
                  }
                />
              </svg>
              {k === "startup" ? (
                <label className="flex cursor-pointer items-center gap-1">
                  <input
                    type="checkbox"
                    checked={showStartup}
                    onChange={onToggleStartup}
                    className="h-3 w-3"
                  />
                  {look.label}
                </label>
              ) : (
                look.label
              )}
            </div>
          );
        })}
    </div>
  );
}
