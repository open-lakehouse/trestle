import {
  Background,
  type ColorMode,
  Controls,
  type Edge,
  MarkerType,
  type Node,
  type NodeTypes,
  ReactFlow,
  ReactFlowProvider,
  useReactFlow,
} from "@xyflow/react";
import { useCallback, useEffect, useMemo } from "react";
import type { GraphDto, GraphNodeDto } from "../types";
import { ModuleNode } from "./nodes/ModuleNode";
import { useTopologyLayout } from "./useTopologyLayout";

const nodeTypes: NodeTypes = {
  module: ModuleNode,
};

export interface MarkitectureCanvasProps {
  /** The plan's dependency graph. */
  graph: GraphDto | undefined;
  /** Currently-selected module id, highlighted in the canvas. */
  selectedId?: string;
  /** Called when a module node is clicked. */
  onSelect?: (node: GraphNodeDto) => void;
  /**
   * Drives ReactFlow's built-in canvas/Controls/Background colors. Defaults to
   * `"system"`; a host with an explicit light/dark toggle can pass its active
   * theme so the canvas tracks the rest of the app.
   */
  colorMode?: ColorMode;
}

/**
 * Render a planned environment as a left-to-right dependency "markitecture".
 * Purely presentational: the app feeds it a live plan graph and Storybook feeds
 * it a fixture — same component, no planner coupling.
 */
export function MarkitectureCanvas(props: MarkitectureCanvasProps) {
  // useReactFlow requires a provider ancestor, so the flow lives inside one.
  return (
    <ReactFlowProvider>
      <MarkitectureFlow {...props} />
    </ReactFlowProvider>
  );
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

  // Animated dashes + an arrowhead give the static dependency graph a sense of
  // startup order. Selecting a node emphasizes its incident edges (in `primary`)
  // and dims the rest. The stroke references the raw HSL-component tokens
  // (authored on the host's :root/.dark), which are theme-reactive.
  const decoratedEdges = useMemo<Edge[]>(
    () =>
      edges.map((e) => {
        const incident =
          !!selectedId && (e.source === selectedId || e.target === selectedId);
        const dimmed = !!selectedId && !incident;
        const stroke = incident
          ? "hsl(var(--primary))"
          : "hsl(var(--muted-foreground))";
        return {
          ...e,
          animated: true,
          markerEnd: {
            type: MarkerType.ArrowClosed,
            width: 16,
            height: 16,
            color: stroke,
          },
          style: {
            stroke,
            strokeWidth: incident ? 2 : 1.5,
            opacity: dimmed ? 0.35 : incident ? 1 : 0.85,
            transition: "opacity 150ms ease, stroke 150ms ease",
          },
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
      void fitView({ padding: 0.2, duration: 200 });
    });
    return () => cancelAnimationFrame(raf);
  }, [nodeKey, fitView]);

  const handleNodeClick = useCallback(
    (_: unknown, node: Node) => {
      const moduleNode = (node.data as { node?: GraphNodeDto }).node;
      if (moduleNode && onSelect) onSelect(moduleNode);
    },
    [onSelect],
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
    </ReactFlow>
  );
}
