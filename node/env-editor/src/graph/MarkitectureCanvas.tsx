import {
  Background,
  type ColorMode,
  Controls,
  type Edge,
  type EdgeTypes,
  MarkerType,
  type Node,
  type NodeTypes,
  Panel,
  ReactFlow,
  ReactFlowProvider,
  useReactFlow,
} from "@xyflow/react";
import { type CSSProperties, useCallback, useEffect, useMemo } from "react";
import type {
  GraphDto,
  GraphNodeDto,
  TopologyEdgeDto,
  TopologyEdgeKind,
} from "../types";
import { ElkEdge } from "./edges/ElkEdge";
import { ClientsNode } from "./nodes/ClientsNode";
import { ComponentNode } from "./nodes/ComponentNode";
import { GatewayNode } from "./nodes/GatewayNode";
import { type TopologyEdgeData, useTopologyLayout } from "./useTopologyLayout";

const nodeTypes: NodeTypes = {
  clients: ClientsNode,
  component: ComponentNode,
  gateway: GatewayNode,
};

const edgeTypes: EdgeTypes = {
  elk: ElkEdge,
};

export interface MarkitectureCanvasProps {
  /** The plan's functional topology. */
  graph: GraphDto | undefined;
  /** Currently-selected component (module) id, highlighted in the canvas. */
  selectedId?: string;
  /** Called when a component (or the gateway) is clicked. */
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
 * on the host's :root/.dark), which are theme-reactive. An animated edge is
 * drawn by ReactFlow as moving 5px dashes.
 */
const EDGE_STYLE: Record<
  TopologyEdgeKind,
  { label: string; stroke: string; dash?: string; animated: boolean }
> = {
  access: {
    label: "Client access",
    stroke: "hsl(var(--muted-foreground))",
    animated: true,
  },
  route: {
    label: "Exposed via gateway",
    stroke: "hsl(var(--primary))",
    animated: true,
  },
  authenticates: {
    label: "Authenticates users",
    stroke: "hsl(var(--status-blocked))",
    dash: "6 4",
    animated: false,
  },
  uses: {
    label: "Uses",
    stroke: "hsl(var(--muted-foreground))",
    animated: false,
  },
};

/** A short label for an edge (the gateway rows already detail routes). */
function edgeLabel(e: TopologyEdgeDto): string | undefined {
  switch (e.kind) {
    case "access":
      return e.host_ports.map((p) => `:${p}`).join(" ");
    case "route":
      return undefined;
    case "authenticates":
      return "sign-in";
    case "uses":
      return `${e.protocol} · ${e.resources.join(", ")}`;
  }
}

/** The full detail of an edge, for its accessible label. */
function edgeTitle(e: TopologyEdgeDto): string {
  switch (e.kind) {
    case "access":
      return `Clients reach the gateway on ${e.host_ports.join(", ")}`;
    case "route":
      return `The gateway exposes ${e.to}${e.gated ? " (requires sign-in)" : ""}`;
    case "authenticates":
      return `The gateway delegates sign-in to ${e.to}; login portal at ${e.portal_prefix}`;
    case "uses":
      return `${e.from} uses ${e.to} (${e.protocol}) for ${e.resources.join(", ")}`;
  }
}

/**
 * Render a planned environment's functional topology as a left-to-right
 * "markitecture": clients → the gateway's exposed surface → the components
 * behind it → the capabilities they use. Purely presentational: the app feeds
 * it a live plan graph and Storybook feeds it a fixture — same component, no
 * planner coupling.
 */
export function MarkitectureCanvas(props: MarkitectureCanvasProps) {
  // useReactFlow requires a provider ancestor, so the flow lives inside one.
  return (
    <ReactFlowProvider>
      <MarkitectureFlow {...props} />
    </ReactFlowProvider>
  );
}

/** The component a node stands for (absent for the clients node). */
function nodeComponent(node: Node): GraphNodeDto | undefined {
  const dto = (node.data as { node?: GraphNodeDto }).node;
  return dto && dto.kind !== "clients" ? dto : undefined;
}

function MarkitectureFlow({
  graph,
  selectedId,
  onSelect,
  colorMode = "system",
}: MarkitectureCanvasProps) {
  const { nodes, edges } = useTopologyLayout(graph);
  const { fitView } = useReactFlow();

  const decoratedNodes = useMemo(
    () => nodes.map((n) => ({ ...n, selected: n.id === selectedId })),
    [nodes, selectedId],
  );

  // Each kind has its own stroke/dash; selecting a component emphasizes its
  // incident edges and dims the rest.
  const decoratedEdges = useMemo<Edge[]>(
    () =>
      edges.map((e) => {
        const { edge } = e.data as TopologyEdgeData;
        const look = EDGE_STYLE[edge.kind];
        const incident =
          !!selectedId && (e.source === selectedId || e.target === selectedId);
        const dimmed = !!selectedId && !incident;
        const style: CSSProperties = {
          stroke: look.stroke,
          strokeWidth: incident ? 2.5 : 1.5,
          strokeDasharray: look.dash,
          opacity: dimmed ? 0.25 : incident ? 1 : 0.85,
          transition: "opacity 150ms ease",
        };
        return {
          ...e,
          animated: look.animated,
          label: edgeLabel(edge),
          ariaLabel: edgeTitle(edge),
          labelStyle: { fontSize: 10, fill: "hsl(var(--muted-foreground))" },
          labelBgStyle: { fill: "hsl(var(--background))", fillOpacity: 0.85 },
          labelBgPadding: [4, 2] as [number, number],
          markerEnd: {
            type: MarkerType.ArrowClosed,
            width: 14,
            height: 14,
            color: look.stroke,
          },
          style,
        };
      }),
    [edges, selectedId],
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
      const component = nodeComponent(node);
      if (component && onSelect) onSelect(component);
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
      edgeTypes={edgeTypes}
      onNodeClick={handleNodeClick}
      colorMode={colorMode}
      fitView
      proOptions={{ hideAttribution: true }}
      minZoom={0.1}
    >
      <Background />
      <Controls showInteractive={false} />
      <Panel position="top-right">
        <Legend kinds={kinds} />
      </Panel>
    </ReactFlow>
  );
}

/** A compact key for the edge kinds present in the graph. */
function Legend({ kinds }: { kinds: Set<TopologyEdgeKind> }) {
  const order: TopologyEdgeKind[] = [
    "access",
    "route",
    "authenticates",
    "uses",
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
                  strokeDasharray={
                    look.dash ?? (look.animated ? "5" : undefined)
                  }
                />
              </svg>
              {look.label}
            </div>
          );
        })}
    </div>
  );
}
