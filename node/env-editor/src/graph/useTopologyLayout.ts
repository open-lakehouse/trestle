// ELK-driven layout for the environment "markitecture". The graph is the
// plan's *functional* topology (see `crates/stack-topology-wasm/src/topology.rs`):
// one node per component, read in request direction — clients → gateway →
// the components it exposes → the capabilities they use — laid out
// left-to-right with ELK's `layered` algorithm (the same engine + tuning the
// sibling headwaters lineage UI uses).
//
// The gateway is drawn as a card listing its exposed surface, one row per
// backend. Each row is an ELK port at a fixed position, so the route to a
// backend leaves from its row and ELK orders the backends to match. Edges are
// drawn along ELK's own orthogonal routes (`ElkEdge`).
//
// The hook is async — ELK resolves to positioned ReactFlow nodes + edges. Until
// it resolves it keeps the previous layout, so the canvas never flashes
// unpositioned nodes. A monotonic `runRef` guards against a stale layout
// overwriting a newer one.

import type { Edge, Node } from "@xyflow/react";
import ELK, { type ElkNode } from "elkjs/lib/elk.bundled.js";
import { useEffect, useRef, useState } from "react";
import type {
  ExposedDto,
  GraphDto,
  GraphNodeDto,
  TopologyEdgeDto,
} from "../types";
import type { Point } from "./edges/ElkEdge";
import type { ComponentNodeData } from "./nodes/ComponentNode";
import type { GatewayNodeData } from "./nodes/GatewayNode";

const elk = new ELK();

// Fixed node boxes. MUST match the components' size classes (`ComponentNode`:
// `h-[92px] w-[240px]`, `ClientsNode`: `h-[64px] w-[160px]`) and the
// `GatewayNode` geometry: ELK positions by these sizes (and places the
// gateway's row ports from them), and ReactFlow anchors handles at the real
// boxes, so a mismatch bends edges.
export const COMPONENT_SIZE = { width: 240, height: 92 };
export const CLIENTS_SIZE = { width: 160, height: 64 };
export const GATEWAY_WIDTH = 300;
export const GATEWAY_HEADER = 52;
export const GATEWAY_ROW = 44;

const LAYOUT_OPTIONS: Record<string, string> = {
  "elk.algorithm": "layered",
  "elk.direction": "RIGHT",
  "elk.layered.spacing.nodeNodeBetweenLayers": "120",
  "elk.spacing.nodeNode": "36",
  "elk.layered.crossingMinimization.strategy": "LAYER_SWEEP",
  // Orthogonal routes are drawn as-is by `ElkEdge`, so they avoid nodes and
  // keep ELK's channels rather than being re-routed by ReactFlow.
  "elk.layered.edgeRouting": "ORTHOGONAL",
  "elk.layered.spacing.edgeEdgeBetweenLayers": "14",
  "elk.layered.spacing.edgeNodeBetweenLayers": "40",
};

/** The gateway's exposed surface for one backend. */
export interface GatewayRow {
  module: string;
  exposes: ExposedDto[];
}

/** The handle / ELK port id of the gateway row for `module`. */
export const routeHandle = (module: string) => `route:${module}`;

/** The `data` a visual edge carries. */
export interface TopologyEdgeData {
  edge: TopologyEdgeDto;
  /** ELK's route (start, bends, end) in flow coordinates. */
  points?: Point[];
  [key: string]: unknown;
}

export interface TopologyFlow {
  nodes: Node[];
  edges: Edge<TopologyEdgeData>[];
}

/** Group the gateway's surface into one row per backend, in surface order. */
export function gatewayRows(gateway: GraphNodeDto): GatewayRow[] {
  const rows: GatewayRow[] = [];
  for (const e of gateway.exposes) {
    const row = rows.find((r) => r.module === e.module);
    if (row) row.exposes.push(e);
    else rows.push({ module: e.module, exposes: [e] });
  }
  return rows;
}

const gatewayHeight = (rows: GatewayRow[]) =>
  GATEWAY_HEADER + rows.length * GATEWAY_ROW;

/** Edges leaving the gateway toward a backend leave from that backend's row. */
function sourceHandle(
  e: TopologyEdgeDto,
  rowsOf: Map<string, GatewayRow[]>,
): string | undefined {
  if (e.kind !== "route" && e.kind !== "authenticates") return undefined;
  const rows = rowsOf.get(e.from);
  return rows?.some((r) => r.module === e.to) ? routeHandle(e.to) : undefined;
}

/**
 * Compute a positioned ReactFlow graph from a plan's functional `GraphDto`.
 * Edges point from caller to callee (`from → to`), so requests flow
 * left→right.
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

    const rowsOf = new Map<string, GatewayRow[]>(
      graph.nodes
        .filter((n) => n.kind === "gateway")
        .map((n) => [n.id, gatewayRows(n)]),
    );

    const children: ElkNode[] = graph.nodes.map((n) => {
      const rows = rowsOf.get(n.id);
      if (rows) {
        // One fixed port per row (right edge, row center) plus the inbound
        // side, so ELK orders the backends like the rows.
        const height = gatewayHeight(rows);
        return {
          id: n.id,
          width: GATEWAY_WIDTH,
          height,
          layoutOptions: { "elk.portConstraints": "FIXED_POS" },
          ports: [
            { id: `${n.id}:in`, x: 0, y: height / 2, width: 1, height: 1 },
            ...rows.map((r, i) => ({
              id: `${n.id}:${routeHandle(r.module)}`,
              x: GATEWAY_WIDTH,
              y: GATEWAY_HEADER + i * GATEWAY_ROW + GATEWAY_ROW / 2,
              width: 1,
              height: 1,
            })),
          ],
        };
      }
      return {
        id: n.id,
        ...(n.kind === "clients" ? CLIENTS_SIZE : COMPONENT_SIZE),
      };
    });

    const elkGraph: ElkNode = {
      id: "root",
      layoutOptions: LAYOUT_OPTIONS,
      children,
      edges: graph.edges.map((e, i) => {
        const handle = sourceHandle(e, rowsOf);
        const toGateway = rowsOf.has(e.to);
        return {
          id: `e${i}`,
          sources: [handle ? `${e.from}:${handle}` : e.from],
          targets: [toGateway ? `${e.to}:in` : e.to],
        };
      }),
    };

    elk
      .layout(elkGraph)
      .then((laidOut) => {
        if (run !== runRef.current) return; // superseded
        const positioned = new Map<string, { x: number; y: number }>();
        for (const child of laidOut.children ?? []) {
          positioned.set(child.id, { x: child.x ?? 0, y: child.y ?? 0 });
        }
        // Flat graph: every edge lives on the root, so its sections are already
        // in root (= flow) coordinates.
        const routes = new Map<string, Point[]>();
        for (const e of laidOut.edges ?? []) {
          const section = e.sections?.[0];
          if (section) {
            routes.set(e.id, [
              section.startPoint,
              ...(section.bendPoints ?? []),
              section.endPoint,
            ]);
          }
        }
        const names: Record<string, string> = Object.fromEntries(
          graph.nodes.map((n) => [n.id, n.display_name ?? n.id]),
        );
        const nodes: Node[] = graph.nodes.map((n) => {
          const position = positioned.get(n.id) ?? { x: 0, y: 0 };
          const rows = rowsOf.get(n.id);
          if (rows) {
            return {
              id: n.id,
              type: "gateway",
              position,
              data: { node: n, rows, names } satisfies GatewayNodeData,
            };
          }
          return {
            id: n.id,
            type: n.kind === "clients" ? "clients" : "component",
            position,
            data: { node: n } satisfies ComponentNodeData,
          };
        });
        const edges: Edge<TopologyEdgeData>[] = graph.edges.map((e, i) => ({
          id: `e${i}`,
          source: e.from,
          target: e.to,
          sourceHandle: sourceHandle(e, rowsOf),
          type: "elk",
          data: { edge: e, points: routes.get(`e${i}`) },
        }));
        setFlow({ nodes, edges });
      })
      .catch(() => {
        if (run === runRef.current) setFlow({ nodes: [], edges: [] });
      });
  }, [graph]);

  return flow;
}
