// ELK-driven layout for the environment "markitecture". The graph is the
// plan's *functional* topology (see `crates/stack-topology-wasm/src/topology.rs`):
// one node per component, read in request direction — clients → gateway →
// the components it exposes → the capabilities they use — laid out
// left-to-right with ELK's `layered` algorithm (the same engine + tuning the
// sibling headwaters lineage UI uses).
//
// The gateway is drawn as a card with one section per surface (listener): the
// platform's Lakehouse API, then each service endpoint (e.g. S3) — locally
// separate ports, hosted separate subdomains — and last its access control:
// the request checks it delegates (sign-in, then authorization). Clients enter
// each surface section through its own port, and each backend row and check
// row leaves through its own port, all at fixed ELK positions so edges start at
// their row and ELK orders the targets to match. Edges are drawn along ELK's
// own orthogonal routes (`ElkEdge`).
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
  SurfaceDto,
  TopologyEdgeDto,
} from "../types";
import type { Point } from "./edges/ElkEdge";
import type { ComponentNodeData } from "./nodes/ComponentNode";
import type { GatewayNodeData } from "./nodes/GatewayNode";

const elk = new ELK();

// Node geometry. MUST match what the nodes render (`NodeHeader`,
// `ComponentNode`, `ClientsNode`, `GatewayNode`): ELK positions by these sizes
// (and places the gateway's row ports from them), and ReactFlow anchors handles
// at the real boxes, so a mismatch bends edges.
export const CLIENTS_SIZE = { width: 200, height: 64 };
// Every card opens with the same header: logo tile, name, role.
export const NODE_HEADER = 52;
// A component card: the header, then one labeled line per fact (see
// `componentFacts`), so its height grows with what it offers and provisions.
export const COMPONENT_WIDTH = 240;
export const FACT_PAD = 8;
export const FACT_LINE = 18;
export const GATEWAY_WIDTH = 300;
export const GATEWAY_SECTION_HEADER = 28;
// A backend row: its title, then one line per exposed endpoint, so its height
// grows with the endpoint count (see `gatewayRowHeight`).
export const GATEWAY_ROW_PAD = 6;
export const GATEWAY_ROW_TITLE = 18;
export const GATEWAY_ENDPOINT_LINE = 16;
// An access-control row: the check's provider and what it does.
export const GATEWAY_CHECK_ROW = 40;

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

/** One backend's endpoints within a gateway surface. */
export interface GatewayRow {
  module: string;
  exposes: ExposedDto[];
}

/** One gateway surface (listener) and its rows, one per backend. */
export interface GatewaySection {
  surface: SurfaceDto;
  rows: GatewayRow[];
}

/** A request check the gateway delegates, in the order it runs. */
export type AccessCheckKind = "authenticates" | "authorizes";

/** One access-control row: the check and the component that performs it. */
export interface AccessCheck {
  kind: AccessCheckKind;
  module: string;
}

/** One labeled line on a component card. */
export interface Fact {
  label: string;
  value: string;
}

const PROVISION_LABEL: Record<string, string> = {
  relational_db: "Databases",
  object_store: "Buckets",
};

/** What a component card lists under its header: the APIs it offers, then
 *  what it provisions, one line per resource kind. Shared by `ComponentNode`
 *  and the layout, so the card's height is the one ELK reserves. */
export function componentFacts(node: GraphNodeDto): Fact[] {
  const facts: Fact[] = [];
  if (node.offers.length > 0) {
    facts.push({ label: "Offers", value: node.offers.join(", ") });
  }
  const byKind = new Map<string, Set<string>>();
  for (const p of node.provisions) {
    const names = byKind.get(p.resource) ?? new Set<string>();
    names.add(p.name);
    byKind.set(p.resource, names);
  }
  for (const [kind, names] of byKind) {
    facts.push({
      label: PROVISION_LABEL[kind] ?? "Provisions",
      value: [...names].join(" · "),
    });
  }
  return facts;
}

/** A component card's height: the header, plus padded fact lines if any. */
export const componentHeight = (node: GraphNodeDto) => {
  const facts = componentFacts(node).length;
  return NODE_HEADER + (facts > 0 ? 2 * FACT_PAD + facts * FACT_LINE : 0);
};

/** A backend row's height: padding, title, and one line per endpoint. MUST be
 *  what `GatewayNode` renders, since ELK places the row's port from it. */
export const gatewayRowHeight = (row: GatewayRow) =>
  2 * GATEWAY_ROW_PAD +
  GATEWAY_ROW_TITLE +
  row.exposes.length * GATEWAY_ENDPOINT_LINE;

/** The handle / ELK port id where clients enter the surface on `port`. */
export const surfaceHandle = (port: number) => `surface:${port}`;

/** The handle / ELK port id of the row for `module` on the surface on `port`. */
export const routeHandle = (port: number, module: string) =>
  `route:${port}:${module}`;

/** The handle / ELK port id of the gateway's access-control row for `kind`. */
export const checkHandle = (kind: AccessCheckKind) => `check:${kind}`;

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

/** Group each gateway surface into one row per backend, in surface order. */
export function gatewaySections(gateway: GraphNodeDto): GatewaySection[] {
  return gateway.surfaces.map((surface) => {
    const rows: GatewayRow[] = [];
    for (const e of surface.exposes) {
      const row = rows.find((r) => r.module === e.module);
      if (row) row.exposes.push(e);
      else rows.push({ module: e.module, exposes: [e] });
    }
    return { surface, rows };
  });
}

const CHECK_ORDER: AccessCheckKind[] = ["authenticates", "authorizes"];

/** The checks gateway `id` delegates (its outgoing authenticates/authorizes
 *  edges), sign-in first. */
export function gatewayChecks(
  id: string,
  edges: TopologyEdgeDto[],
): AccessCheck[] {
  const checks: AccessCheck[] = [];
  for (const e of edges) {
    if (
      (e.kind === "authenticates" || e.kind === "authorizes") &&
      e.from === id
    ) {
      checks.push({ kind: e.kind, module: e.to });
    }
  }
  return checks.sort(
    (a, b) => CHECK_ORDER.indexOf(a.kind) - CHECK_ORDER.indexOf(b.kind),
  );
}

/** The gateway's ELK ports, at the same offsets `GatewayNode` renders: one
 *  inbound port per section header (left), one outbound per backend row and
 *  per access-control row (right). */
function gatewayPorts(
  id: string,
  sections: GatewaySection[],
  checks: AccessCheck[],
) {
  const ports: { id: string; x: number; y: number; width: 1; height: 1 }[] = [];
  let y = NODE_HEADER;
  for (const { surface, rows } of sections) {
    ports.push({
      id: `${id}:${surfaceHandle(surface.host_port)}`,
      x: 0,
      y: y + GATEWAY_SECTION_HEADER / 2,
      width: 1,
      height: 1,
    });
    y += GATEWAY_SECTION_HEADER;
    for (const row of rows) {
      ports.push({
        id: `${id}:${routeHandle(surface.host_port, row.module)}`,
        x: GATEWAY_WIDTH,
        y: y + gatewayRowHeight(row) / 2,
        width: 1,
        height: 1,
      });
      y += gatewayRowHeight(row);
    }
  }
  if (checks.length > 0) {
    y += GATEWAY_SECTION_HEADER;
    for (const check of checks) {
      ports.push({
        id: `${id}:${checkHandle(check.kind)}`,
        x: GATEWAY_WIDTH,
        y: y + GATEWAY_CHECK_ROW / 2,
        width: 1,
        height: 1,
      });
      y += GATEWAY_CHECK_ROW;
    }
  }
  return { ports, height: y };
}

/** The gateway handles an edge attaches to, if it touches a gateway: routes
 *  leave from their backend's row, checks from their access-control row, and
 *  access enters its surface's header. */
function gatewayHandles(
  e: TopologyEdgeDto,
  gateways: Set<string>,
): { source?: string; target?: string } {
  switch (e.kind) {
    case "route":
      return gateways.has(e.from)
        ? { source: routeHandle(e.host_port, e.to) }
        : {};
    case "authenticates":
    case "authorizes":
      return gateways.has(e.from) ? { source: checkHandle(e.kind) } : {};
    case "access":
      return gateways.has(e.to) ? { target: surfaceHandle(e.host_port) } : {};
    default:
      return {};
  }
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

    const sectionsOf = new Map<string, GatewaySection[]>(
      graph.nodes
        .filter((n) => n.kind === "gateway")
        .map((n) => [n.id, gatewaySections(n)]),
    );
    const gateways = new Set(sectionsOf.keys());
    const checksOf = new Map<string, AccessCheck[]>(
      [...gateways].map((id) => [id, gatewayChecks(id, graph.edges)]),
    );

    const children: ElkNode[] = graph.nodes.map((n) => {
      const sections = sectionsOf.get(n.id);
      if (sections) {
        const { ports, height } = gatewayPorts(
          n.id,
          sections,
          checksOf.get(n.id) ?? [],
        );
        return {
          id: n.id,
          width: GATEWAY_WIDTH,
          height,
          layoutOptions: { "elk.portConstraints": "FIXED_POS" },
          ports,
        };
      }
      return {
        id: n.id,
        ...(n.kind === "clients"
          ? CLIENTS_SIZE
          : { width: COMPONENT_WIDTH, height: componentHeight(n) }),
      };
    });

    const elkGraph: ElkNode = {
      id: "root",
      layoutOptions: LAYOUT_OPTIONS,
      children,
      edges: graph.edges.map((e, i) => {
        const { source, target } = gatewayHandles(e, gateways);
        return {
          id: `e${i}`,
          sources: [source ? `${e.from}:${source}` : e.from],
          targets: [target ? `${e.to}:${target}` : e.to],
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
        const roles: Record<string, string | null | undefined> =
          Object.fromEntries(graph.nodes.map((n) => [n.id, n.role]));
        const nodes: Node[] = graph.nodes.map((n) => {
          const position = positioned.get(n.id) ?? { x: 0, y: 0 };
          const sections = sectionsOf.get(n.id);
          if (sections) {
            // Only rows a route leaves from get a handle (the identity
            // provider's portal row is drawn, but sign-in leaves from its
            // access-control row).
            const anchored = graph.edges.flatMap((e) =>
              e.kind === "route" && e.from === n.id
                ? [routeHandle(e.host_port, e.to)]
                : [],
            );
            return {
              id: n.id,
              type: "gateway",
              position,
              data: {
                node: n,
                sections,
                checks: checksOf.get(n.id) ?? [],
                anchored,
                names,
                roles,
              } satisfies GatewayNodeData,
            };
          }
          return {
            id: n.id,
            type: n.kind === "clients" ? "clients" : "component",
            position,
            data: { node: n } satisfies ComponentNodeData,
          };
        });
        const edges: Edge<TopologyEdgeData>[] = graph.edges.map((e, i) => {
          const { source, target } = gatewayHandles(e, gateways);
          return {
            id: `e${i}`,
            source: e.from,
            target: e.to,
            sourceHandle: source,
            targetHandle: target,
            type: "elk",
            data: { edge: e, points: routes.get(`e${i}`) },
          };
        });
        setFlow({ nodes, edges });
      })
      .catch(() => {
        if (run === runRef.current) setFlow({ nodes: [], edges: [] });
      });
  }, [graph]);

  return flow;
}
