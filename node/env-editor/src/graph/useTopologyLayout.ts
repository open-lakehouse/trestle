// ELK-driven layout for the environment "markitecture". The graph is the
// plan's *runtime* topology (compose services + gateway wiring, see
// `crates/stack-topology-wasm/src/topology.rs`), read in request direction:
// host → gateway → apps → backing stores, laid out left-to-right with ELK's
// `layered` algorithm (the same engine + tuning the sibling headwaters lineage
// UI uses).
//
// Services are the leaf nodes. A module that runs more than one long-running
// service (postgres: db + pgweb; rustfs: rustfs + sts-shim) becomes a compound
// group around them; a single-service module is just its one node, so the
// canvas stays as compact as a module graph.
//
// The hook is async — ELK resolves to positioned ReactFlow nodes + edges. Until
// it resolves it keeps the previous layout, so the canvas never flashes
// unpositioned nodes. A monotonic `runRef` guards against a stale layout
// overwriting a newer one.

import type { Edge, Node } from "@xyflow/react";
import ELK, { type ElkNode } from "elkjs/lib/elk.bundled.js";
import { useEffect, useRef, useState } from "react";
import type {
  GraphDto,
  GraphNodeDto,
  ServiceNodeDto,
  TopologyEdgeDto,
  TopologyEdgeKind,
} from "../types";
import type { ModuleGroupNodeData } from "./nodes/ModuleGroupNode";
import type { ServiceNodeData } from "./nodes/ServiceNode";

const elk = new ELK();

/** The id of the synthetic host/browser node (Rust `HOST_NODE_ID`). */
export const HOST_NODE_ID = "@host";

// Fixed node boxes. MUST match the components' size classes (`ServiceNode`:
// `h-[92px] w-[240px]`, `HostNode`: `h-[64px] w-[160px]`): ELK positions by
// these sizes and ReactFlow anchors handles at the real box center, so a
// mismatch bends same-row edges.
export const SERVICE_SIZE = { width: 240, height: 92 };
export const HOST_SIZE = { width: 160, height: 64 };
// A group's header band (MUST match `ModuleGroupNode`'s header height) plus
// the inner padding around its children.
export const GROUP_HEADER = 36;
const GROUP_PAD = 16;

const LAYOUT_OPTIONS: Record<string, string> = {
  "elk.algorithm": "layered",
  "elk.direction": "RIGHT",
  "elk.hierarchyHandling": "INCLUDE_CHILDREN",
  "elk.layered.spacing.nodeNodeBetweenLayers": "140",
  "elk.spacing.nodeNode": "40",
  "elk.layered.crossingMinimization.strategy": "LAYER_SWEEP",
  "elk.layered.edgeRouting": "SPLINES",
  "elk.layered.spacing.edgeNodeBetweenLayers": "40",
};

const GROUP_OPTIONS: Record<string, string> = {
  "elk.padding": `[top=${GROUP_HEADER + GROUP_PAD},left=${GROUP_PAD},bottom=${GROUP_PAD},right=${GROUP_PAD}]`,
  "elk.spacing.nodeNode": "24",
};

/**
 * Which kind wins when several typed edges join the same pair of services:
 * the visual edge takes the style of the most significant one and lists every
 * label. E.g. envoy → authelia is both a startup gate and the auth check.
 */
const KIND_PRIORITY: TopologyEdgeKind[] = [
  "authz",
  "route",
  "emulated",
  "ingress",
  "startup",
];

/** The `data` a visual edge carries: every typed edge it stands for. */
export interface TopologyEdgeData {
  /** The most significant kind among `edges` (drives the style). */
  kind: TopologyEdgeKind;
  edges: TopologyEdgeDto[];
  /** Edges in the opposite direction folded onto this one (a client calling
   *  back into the gateway that routes to it), drawn as a second arrowhead. */
  back: TopologyEdgeDto[];
  [key: string]: unknown;
}

export interface TopologyFlow {
  nodes: Node[];
  edges: Edge<TopologyEdgeData>[];
}

/** The id of a module's group node (distinct from any service id). */
const groupId = (moduleId: string) => `module:${moduleId}`;

/**
 * Merge the typed edges between each (from, to) pair into one visual edge,
 * styled by the most significant kind. Startup edges are optional:
 * `showStartup = false` drops pairs that are *only* startup gates.
 *
 * A client calling back into a gateway that routes to it (UC → envoy for the
 * emulated AWS hostnames) closes a cycle with that route. Drawn separately it
 * loops around the canvas and its label collides with the route's; ELK's cycle
 * breaker may also flip the route. So such a back-edge is folded onto the
 * route as `back` (a second arrowhead + label) and never shapes the layers.
 */
export function mergeEdges(
  edges: TopologyEdgeDto[],
  showStartup: boolean,
): { from: string; to: string; data: TopologyEdgeData }[] {
  const pairKey = (from: string, to: string) => `${from}\u0000${to}`;
  const routed = new Set(
    edges.filter((e) => e.kind === "route").map((e) => pairKey(e.from, e.to)),
  );
  const byPair = new Map<string, TopologyEdgeDto[]>();
  const backs = new Map<string, TopologyEdgeDto[]>();
  for (const e of edges) {
    const reverse = pairKey(e.to, e.from);
    if (e.kind !== "startup" && routed.has(reverse)) {
      const list = backs.get(reverse);
      if (list) list.push(e);
      else backs.set(reverse, [e]);
      continue;
    }
    const key = pairKey(e.from, e.to);
    const list = byPair.get(key);
    if (list) list.push(e);
    else byPair.set(key, [e]);
  }
  const merged: { from: string; to: string; data: TopologyEdgeData }[] = [];
  for (const [key, list] of byPair) {
    const kind =
      KIND_PRIORITY.find((k) => list.some((e) => e.kind === k)) ?? "startup";
    if (kind === "startup" && !showStartup) continue;
    merged.push({
      from: list[0].from,
      to: list[0].to,
      data: { kind, edges: list, back: backs.get(key) ?? [] },
    });
  }
  return merged;
}

/**
 * Compute a positioned ReactFlow graph from a plan's runtime `GraphDto`. Edges
 * point from caller to callee (`from → to`), so requests flow left→right.
 */
export function useTopologyLayout(
  graph: GraphDto | undefined,
  showStartup = true,
): TopologyFlow {
  const [flow, setFlow] = useState<TopologyFlow>({ nodes: [], edges: [] });
  const runRef = useRef(0);

  useEffect(() => {
    const run = ++runRef.current;
    if (!graph || graph.services.length === 0) {
      setFlow({ nodes: [], edges: [] });
      return;
    }

    const modules = new Map<string, GraphNodeDto>(
      graph.nodes.map((m) => [m.id, m]),
    );
    const byModule = new Map<string, ServiceNodeDto[]>();
    for (const s of graph.services) {
      if (!s.module) continue;
      const list = byModule.get(s.module);
      if (list) list.push(s);
      else byModule.set(s.module, [s]);
    }
    const grouped = new Set(
      [...byModule].filter(([, list]) => list.length > 1).map(([id]) => id),
    );

    const leaf = (s: ServiceNodeDto): ElkNode => ({
      id: s.id,
      ...(s.kind === "host" ? HOST_SIZE : SERVICE_SIZE),
    });

    // Top level: ungrouped services in plan order, with each group inserted at
    // its first service's position.
    const children: ElkNode[] = [];
    const placedGroups = new Set<string>();
    for (const s of graph.services) {
      if (s.module && grouped.has(s.module)) {
        if (placedGroups.has(s.module)) continue;
        placedGroups.add(s.module);
        children.push({
          id: groupId(s.module),
          layoutOptions: GROUP_OPTIONS,
          children: (byModule.get(s.module) ?? []).map(leaf),
        });
      } else {
        children.push(leaf(s));
      }
    }

    const merged = mergeEdges(graph.edges, showStartup);
    const elkGraph: ElkNode = {
      id: "root",
      layoutOptions: LAYOUT_OPTIONS,
      children,
      edges: merged.map((e, i) => ({
        id: `e${i}`,
        sources: [e.from],
        targets: [e.to],
      })),
    };

    elk
      .layout(elkGraph)
      .then((laidOut) => {
        if (run !== runRef.current) return; // superseded
        const nodes: Node[] = [];
        const services = new Map(graph.services.map((s) => [s.id, s]));
        // ReactFlow needs a parent before its children; ELK child coordinates
        // are already relative to their parent, matching `parentId` semantics.
        for (const child of laidOut.children ?? []) {
          const position = { x: child.x ?? 0, y: child.y ?? 0 };
          if (child.children) {
            const moduleId = child.id.slice("module:".length);
            const module = modules.get(moduleId);
            if (!module) continue;
            nodes.push({
              id: child.id,
              type: "moduleGroup",
              position,
              width: child.width,
              height: child.height,
              data: { module } satisfies ModuleGroupNodeData,
            });
            for (const grandchild of child.children) {
              const service = services.get(grandchild.id);
              if (!service) continue;
              nodes.push({
                id: service.id,
                type: "service",
                parentId: child.id,
                extent: "parent",
                position: { x: grandchild.x ?? 0, y: grandchild.y ?? 0 },
                data: {
                  service,
                  module,
                  inGroup: true,
                } satisfies ServiceNodeData,
              });
            }
          } else {
            const service = services.get(child.id);
            if (!service) continue;
            nodes.push({
              id: service.id,
              type: service.kind === "host" ? "host" : "service",
              position,
              data: {
                service,
                module: service.module
                  ? modules.get(service.module)
                  : undefined,
                inGroup: false,
              } satisfies ServiceNodeData,
            });
          }
        }
        const edges: Edge<TopologyEdgeData>[] = merged.map((e, i) => ({
          id: `e${i}`,
          source: e.from,
          target: e.to,
          type: "smoothstep",
          data: e.data,
        }));
        setFlow({ nodes, edges });
      })
      .catch(() => {
        if (run === runRef.current) setFlow({ nodes: [], edges: [] });
      });
  }, [graph, showStartup]);

  return flow;
}
