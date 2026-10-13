import {
  Background,
  BackgroundVariant,
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
import {
  type CSSProperties,
  type ReactNode,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import type {
  GraphDto,
  GraphNodeDto,
  TopologyEdgeDto,
  TopologyEdgeKind,
} from "../types";
import {
  type CanvasActions,
  CanvasContext,
  type CanvasContextValue,
} from "./canvasActions";
import { ElkEdge } from "./edges/ElkEdge";
import { ClientsNode } from "./nodes/ClientsNode";
import { CloudGroupNode } from "./nodes/CloudGroupNode";
import { ComponentNode } from "./nodes/ComponentNode";
import { GatewayNode } from "./nodes/GatewayNode";
import { roleStyle } from "./roleTheme";
import { type TopologyEdgeData, useTopologyLayout } from "./useTopologyLayout";

const nodeTypes: NodeTypes = {
  clients: ClientsNode,
  cloud: CloudGroupNode,
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
  /**
   * Editing actions. When given, nodes show a hover action bar (configure /
   * swap / remove) and double-clicking a node configures it; without them the
   * canvas is a read-only diagram.
   */
  actions?: CanvasActions;
  /** Called when the empty canvas is clicked. */
  onDeselect?: () => void;
  /** Pan to a node; bump `seq` to pan again to the same one. */
  focus?: { id: string; seq: number };
  /** Overlays rendered inside the flow (e.g. `<Panel>`s); they may use `useReactFlow`. */
  children?: ReactNode;
}

/** How long the pointer may travel from a node onto its action bar. */
const HOVER_GRACE_MS = 200;
/** How long a newly planned node stays highlighted. */
const FRESH_MS = 1600;

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
  authorizes: {
    label: "Authorizes requests",
    stroke: "hsl(var(--status-blocked))",
    dash: "2 4",
    animated: false,
  },
  uses: {
    label: "Uses",
    stroke: "hsl(var(--muted-foreground))",
    animated: false,
  },
};

/** A short label for an edge. The cards already detail the rest: the gateway
 *  rows list routes, and a provider lists what it offers. */
function edgeLabel(e: TopologyEdgeDto): string | undefined {
  switch (e.kind) {
    case "access":
      return `:${e.host_port}`;
    case "route":
      return undefined;
    case "authenticates":
      return "sign-in";
    case "authorizes":
      return "policy";
    case "uses":
      // A hard requirement names no resources: label it with the capability used.
      return e.resources.length > 0
        ? e.resources.join(", ")
        : roleStyle(e.role).label;
  }
}

/** The full detail of an edge, for its accessible label. */
function edgeTitle(e: TopologyEdgeDto): string {
  switch (e.kind) {
    case "access":
      return `Clients reach the gateway's surface on :${e.host_port}`;
    case "route":
      return `The gateway exposes ${e.to} on :${e.host_port}${e.gated ? " (requires sign-in)" : ""}`;
    case "authenticates":
      return `The gateway delegates sign-in to ${e.to}; login portal at ${e.portal_prefix}`;
    case "authorizes":
      return `The gateway asks ${e.to} to authorize each signed-in request on :${e.host_port}`;
    case "uses":
      return e.resources.length > 0
        ? `${e.from} uses ${e.to} (${e.protocol}) for ${e.resources.join(", ")}`
        : `${e.from} uses ${e.to} (${e.protocol}) as its ${roleStyle(e.role).label.toLowerCase()}`;
  }
}

/**
 * Render a planned environment's functional topology as a left-to-right
 * "markitecture": clients → the gateway's exposed surface → the components
 * behind it → the capabilities they use. Presentational: the app feeds it a
 * live plan graph and Storybook feeds it a fixture — same component, no
 * planner coupling. Editing is opt-in through `actions`, which the host
 * implements.
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
  actions,
  onDeselect,
  focus,
  children,
}: MarkitectureCanvasProps) {
  const { nodes, edges } = useTopologyLayout(graph);
  const { fitView, getInternalNode, getZoom, setCenter } = useReactFlow();
  const [hoveredId, setHoveredId] = useState<string | undefined>();
  const leaveTimer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const fresh = useFreshNodes(nodes);

  const hover = useCallback((id: string) => {
    clearTimeout(leaveTimer.current);
    setHoveredId(id);
  }, []);
  const leave = useCallback(() => {
    clearTimeout(leaveTimer.current);
    leaveTimer.current = setTimeout(
      () => setHoveredId(undefined),
      HOVER_GRACE_MS,
    );
  }, []);
  useEffect(() => () => clearTimeout(leaveTimer.current), []);

  const context = useMemo<CanvasContextValue | null>(
    () => (actions ? { actions, hoveredId, hover, leave } : null),
    [actions, hoveredId, hover, leave],
  );

  const decoratedNodes = useMemo(
    () =>
      nodes.map((n) => ({
        ...n,
        selected: n.id === selectedId,
        className: fresh.has(n.id)
          ? "rounded-lg ring-4 ring-primary/40 animate-pulse"
          : undefined,
      })),
    [nodes, selectedId, fresh],
  );

  // Each kind has its own stroke/dash; selecting a component emphasizes its
  // incident edges and dims the rest.
  const decoratedEdges = useMemo<Edge[]>(
    () =>
      edges.map((e) => {
        const { edge, resource } = e.data as TopologyEdgeData;
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
          // An edge landing on a resource row needs no label: the row names it.
          label: resource ? undefined : edgeLabel(edge),
          ariaLabel: resource
            ? `${edge.from} uses ${resource} on ${edge.to}`
            : edgeTitle(edge),
          labelStyle: {
            fontSize: 10,
            fontWeight: 500,
            fill: "hsl(var(--muted-foreground))",
          },
          labelBgStyle: {
            fill: "hsl(var(--card))",
            stroke: "hsl(var(--border))",
            strokeWidth: 1,
          },
          labelBgPadding: [6, 3] as [number, number],
          labelBgBorderRadius: 4,
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

  // Pan to a requested node at the current zoom.
  const focusSeq = focus?.seq;
  const focusId = focus?.id;
  useEffect(() => {
    if (focusSeq === undefined || !focusId) return;
    const node = getInternalNode(focusId);
    if (!node) return;
    const width = node.measured?.width ?? 0;
    const height = node.measured?.height ?? 0;
    // Absolute: a node inside a group (an external cloud) is positioned
    // relative to it.
    const { x, y } = node.internals.positionAbsolute;
    void setCenter(x + width / 2, y + height / 2, {
      zoom: getZoom(),
      duration: 300,
    });
  }, [focusSeq, focusId, getInternalNode, getZoom, setCenter]);

  const handleNodeClick = useCallback(
    (_: unknown, node: Node) => {
      const component = nodeComponent(node);
      if (component && onSelect) onSelect(component);
    },
    [onSelect],
  );

  const handleNodeDoubleClick = useCallback(
    (_: unknown, node: Node) => {
      const component = nodeComponent(node);
      if (component && actions) actions.configure(component.id);
    },
    [actions],
  );

  const handleNodeEnter = useCallback(
    (_: unknown, node: Node) => {
      if (nodeComponent(node)) hover(node.id);
    },
    [hover],
  );

  const kinds = useMemo(
    () => new Set(graph?.edges.map((e) => e.kind) ?? []),
    [graph],
  );

  return (
    <CanvasContext.Provider value={context}>
      <ReactFlow
        nodes={decoratedNodes}
        edges={decoratedEdges}
        nodeTypes={nodeTypes}
        edgeTypes={edgeTypes}
        onNodeClick={handleNodeClick}
        onNodeDoubleClick={handleNodeDoubleClick}
        onNodeMouseEnter={actions ? handleNodeEnter : undefined}
        onNodeMouseLeave={actions ? leave : undefined}
        onPaneClick={onDeselect}
        // Removal goes through the editor's actions, not React Flow's own
        // delete handling.
        deleteKeyCode={null}
        zoomOnDoubleClick={false}
        colorMode={colorMode}
        fitView
        proOptions={{ hideAttribution: true }}
        minZoom={0.1}
      >
        <Background
          variant={BackgroundVariant.Dots}
          gap={20}
          size={1}
          color="hsl(var(--muted-foreground) / 0.35)"
        />
        <Controls showInteractive={false} />
        <Panel position="top-right">
          <Legend kinds={kinds} />
        </Panel>
        {children}
      </ReactFlow>
    </CanvasContext.Provider>
  );
}

/**
 * The ids of nodes that just appeared in the layout (not on the first
 * layout), cleared after FRESH_MS, so a newly added component stands out.
 */
function useFreshNodes(nodes: Node[]): Set<string> {
  const [fresh, setFresh] = useState<Set<string>>(() => new Set());
  const seen = useRef<Set<string> | null>(null);

  useEffect(() => {
    if (nodes.length === 0) return;
    const ids = new Set(nodes.map((n) => n.id));
    const previous = seen.current;
    seen.current = ids;
    if (!previous) return;
    const added = [...ids].filter((id) => !previous.has(id));
    if (added.length > 0) setFresh(new Set(added));
  }, [nodes]);

  useEffect(() => {
    if (fresh.size === 0) return;
    const timer = setTimeout(() => setFresh(new Set()), FRESH_MS);
    return () => clearTimeout(timer);
  }, [fresh]);

  return fresh;
}

/** A compact key for the edge kinds present in the graph. */
function Legend({ kinds }: { kinds: Set<TopologyEdgeKind> }) {
  const order: TopologyEdgeKind[] = [
    "access",
    "route",
    "authenticates",
    "authorizes",
    "uses",
  ];
  return (
    <div className="rounded-lg border border-border bg-card/95 px-3 py-2 text-[11px] text-muted-foreground shadow-sm">
      <div className="pb-1 text-[10px] font-semibold uppercase tracking-wide">
        Connections
      </div>
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
