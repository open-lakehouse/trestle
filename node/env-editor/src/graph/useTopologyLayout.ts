// ELK-driven layout for the environment "markitecture". The graph reads
// dependencies → dependents, so we lay it out left-to-right with ELK's `layered`
// algorithm (the same engine + tuning the sibling headwaters lineage UI uses).
//
// The hook is async — ELK resolves to positioned ReactFlow nodes + edges. Until
// it resolves it keeps the previous layout, so the canvas never flashes
// unpositioned nodes. A monotonic `runRef` guards against a stale layout
// overwriting a newer one.

import type { Edge, Node } from "@xyflow/react";
import ELK, { type ElkNode } from "elkjs/lib/elk.bundled.js";
import { useEffect, useRef, useState } from "react";
import type { GraphDto } from "../types";
import type { ModuleNodeData } from "./nodes/ModuleNode";

const elk = new ELK();

// One fixed node box. MUST match ModuleNode's `h-[92px] w-[240px]`: ELK positions
// by this size and ReactFlow anchors handles at the real box center, so a
// mismatch bends same-row edges.
const SIZE = { width: 240, height: 92 };

const LAYOUT_OPTIONS: Record<string, string> = {
  "elk.algorithm": "layered",
  "elk.direction": "RIGHT",
  "elk.layered.spacing.nodeNodeBetweenLayers": "120",
  "elk.spacing.nodeNode": "40",
  "elk.layered.crossingMinimization.strategy": "LAYER_SWEEP",
  "elk.layered.edgeRouting": "SPLINES",
  "elk.layered.spacing.edgeNodeBetweenLayers": "40",
};

export interface TopologyFlow {
  nodes: Node[];
  edges: Edge[];
}

/**
 * Compute a positioned ReactFlow graph from a plan's dependency `GraphDto`.
 *
 * A plan edge is `{ from, to }` meaning `from` depends on `to` (so `to` starts
 * first). For a dependencies-first left→right layout we orient the visual edge
 * `to → from`: the dependency sits to the left of its dependent, and the arrow
 * points along startup order.
 */
export function useTopologyLayout(graph: GraphDto | undefined): TopologyFlow {
  const [flow, setFlow] = useState<TopologyFlow>({ nodes: [], edges: [] });
  const runRef = useRef(0);

  useEffect(() => {
    const run = ++runRef.current;
    if (!graph || graph.nodes.length === 0) {
      setFlow({ nodes: [], edges: [] });
      return;
    }

    const elkGraph: ElkNode = {
      id: "root",
      layoutOptions: LAYOUT_OPTIONS,
      children: graph.nodes.map((n) => ({
        id: n.id,
        width: SIZE.width,
        height: SIZE.height,
      })),
      edges: graph.edges.map((e, i) => ({
        id: `e${i}`,
        // dependency (to) → dependent (from): left-to-right startup order.
        sources: [e.to],
        targets: [e.from],
      })),
    };

    elk
      .layout(elkGraph)
      .then((laidOut) => {
        if (run !== runRef.current) return; // superseded
        const positioned = new Map<string, { x: number; y: number }>();
        for (const child of laidOut.children ?? []) {
          positioned.set(child.id, { x: child.x ?? 0, y: child.y ?? 0 });
        }
        const nodes: Node[] = graph.nodes.map((n) => ({
          id: n.id,
          type: "module",
          position: positioned.get(n.id) ?? { x: 0, y: 0 },
          data: { node: n } satisfies ModuleNodeData,
        }));
        const edges: Edge[] = graph.edges.map((e, i) => ({
          id: `e${i}`,
          source: e.to,
          target: e.from,
          type: "smoothstep",
        }));
        setFlow({ nodes, edges });
      })
      .catch(() => {
        if (run === runRef.current) setFlow({ nodes: [], edges: [] });
      });
  }, [graph]);

  return flow;
}
