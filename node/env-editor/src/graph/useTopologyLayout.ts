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
// their row and ELK orders the targets to match. A provider's card lists what
// it provisions, one row per database / bucket, and a consumer's edge enters
// the row of each resource it uses. Edges are drawn along ELK's own orthogonal
// routes (`ElkEdge`).
//
// Resources an external cloud provides (a managed database, a cloud object
// store) are drawn inside one dashed frame per cloud: an ELK compound node laid
// out with the rest of the graph (hierarchy included), and a ReactFlow parent
// node whose children are positioned relative to it.
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
  ProvisionedDto,
  SurfaceDto,
  TopologyEdgeDto,
} from "../types";
import type { Point } from "./edges/ElkEdge";
import type { CloudGroupNodeData } from "./nodes/CloudGroupNode";
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
// A provider lists what it provisions in one section per resource kind: a
// header, then one row per database / bucket, each with its own port so a
// consumer's edge lands on the resource it uses.
export const RESOURCE_SECTION_HEADER = 24;
export const RESOURCE_ROW = 24;
export const GATEWAY_WIDTH = 300;
export const GATEWAY_SECTION_HEADER = 28;
// A backend row: its title, then one line per exposed endpoint, so its height
// grows with the endpoint count (see `gatewayRowHeight`).
export const GATEWAY_ROW_PAD = 6;
export const GATEWAY_ROW_TITLE = 18;
export const GATEWAY_ENDPOINT_LINE = 16;
// An access-control row: the check's provider and what it does.
export const GATEWAY_CHECK_ROW = 40;
// An external-cloud frame: its title strip, then padding around its cards.
export const CLOUD_GROUP_HEADER = 32;
export const CLOUD_GROUP_PAD = 16;

/** The canvas id of the frame around the external cloud `cloud`'s resources. */
export const cloudGroupId = (cloud: string) => `@cloud:${cloud}`;

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

/** The section title for each resource kind a provider provisions, when the
 *  provider doesn't name it. */
export const PROVISION_LABEL: Record<string, string> = {
  relational_db: "Databases",
  object_store: "Buckets",
};

/** The section title for a resource kind, in the provider's own terms (Azurite
 *  provisions "Containers", not "Buckets"). */
export function provisionLabel(node: GraphNodeDto, kind: string): string {
  const plural = node.resource_nouns.find((n) => n.kind === kind)?.plural;
  return plural
    ? plural.charAt(0).toUpperCase() + plural.slice(1)
    : (PROVISION_LABEL[kind] ?? "Resources");
}

/** One resource kind a provider provisions, and its resources in init order. */
export interface ResourceSection {
  kind: string;
  items: ProvisionedDto[];
}

/** What a component card lists under its header: the APIs it offers. Shared
 *  by `ComponentNode` and the layout, so the card's height is the one ELK
 *  reserves. */
export function componentFacts(node: GraphNodeDto): Fact[] {
  return node.offers.length > 0
    ? [{ label: "Offers", value: node.offers.join(", ") }]
    : [];
}

/** A provider's resources, one section per kind, in first-provisioned order. */
export function resourceSections(node: GraphNodeDto): ResourceSection[] {
  const sections: ResourceSection[] = [];
  for (const p of node.provisions) {
    const section = sections.find((s) => s.kind === p.resource);
    if (section) {
      if (!section.items.some((i) => i.name === p.name)) section.items.push(p);
    } else {
      sections.push({ kind: p.resource, items: [p] });
    }
  }
  return sections;
}

/** The height of a component card's facts block (0 without facts). */
export const factsHeight = (node: GraphNodeDto) => {
  const facts = componentFacts(node).length;
  return facts > 0 ? 2 * FACT_PAD + facts * FACT_LINE : 0;
};

/** A component card's height: the header, its facts, then its resource
 *  sections. MUST be what `ComponentNode` renders. */
export const componentHeight = (node: GraphNodeDto) =>
  NODE_HEADER +
  factsHeight(node) +
  resourceSections(node).reduce(
    (h, s) => h + RESOURCE_SECTION_HEADER + s.items.length * RESOURCE_ROW,
    0,
  );

/** A component card's ELK ports, at the offsets `ComponentNode` renders: the
 *  generic in/out ports level with the header, and one inbound port per
 *  resource row. */
function componentPorts(node: GraphNodeDto) {
  const port = (handle: string, x: number, y: number) => ({
    id: `${node.id}:${handle}`,
    x,
    y,
    width: 1 as const,
    height: 1 as const,
  });
  const ports = [
    port(COMPONENT_IN, 0, NODE_HEADER / 2),
    port(COMPONENT_OUT, COMPONENT_WIDTH, NODE_HEADER / 2),
  ];
  let y = NODE_HEADER + factsHeight(node);
  for (const section of resourceSections(node)) {
    y += RESOURCE_SECTION_HEADER;
    for (const item of section.items) {
      ports.push(
        port(resourceHandle(section.kind, item.name), 0, y + RESOURCE_ROW / 2),
      );
      y += RESOURCE_ROW;
    }
  }
  return ports;
}

/** A component card's generic inbound / outbound handles (level with its header). */
export const COMPONENT_IN = "in";
export const COMPONENT_OUT = "out";

/** The handle / ELK port id of a provider's row for resource `name` of `kind`. */
export const resourceHandle = (kind: string, name: string) =>
  `res:${kind}:${name}`;

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
  /** For a `uses` edge split per resource: the database / bucket it lands on. */
  resource?: string;
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

/** The handles an edge attaches to. At the gateway, routes leave from their
 *  backend's row, checks from their access-control row, and access enters its
 *  surface's header. At a component, edges leave from its out handle and
 *  enter its in handle, unless `resource` names the provider row it uses. */
function edgeHandles(
  e: TopologyEdgeDto,
  gateways: Set<string>,
  components: Set<string>,
  resource?: string,
): { source?: string; target?: string } {
  const handles: { source?: string; target?: string } = {};
  if (gateways.has(e.from)) {
    if (e.kind === "route") handles.source = routeHandle(e.host_port, e.to);
    else if (e.kind === "authenticates" || e.kind === "authorizes") {
      handles.source = checkHandle(e.kind);
    }
  } else if (components.has(e.from)) {
    handles.source = COMPONENT_OUT;
  }
  if (gateways.has(e.to)) {
    if (e.kind === "access") handles.target = surfaceHandle(e.host_port);
  } else if (components.has(e.to)) {
    handles.target =
      e.kind === "uses" && resource
        ? resourceHandle(e.role, resource)
        : COMPONENT_IN;
  }
  return handles;
}

/** One drawn edge: a topology edge, or one resource of a `uses` edge whose
 *  provider lists that resource in its card. */
interface VisualEdge {
  id: string;
  edge: TopologyEdgeDto;
  resource?: string;
}

/** Split each `uses` edge into one edge per resource the provider's card
 *  lists, so each lands on its row; everything else is drawn as-is. */
function visualEdges(graph: GraphDto): VisualEdge[] {
  const listed = new Map<string, Set<string>>(
    graph.nodes.map((n) => [
      n.id,
      new Set(n.provisions.map((p) => `${p.resource}:${p.name}`)),
    ]),
  );
  return graph.edges.flatMap((edge, i): VisualEdge[] => {
    if (edge.kind === "uses") {
      const rows = edge.resources.filter((r) =>
        listed.get(edge.to)?.has(`${edge.role}:${r}`),
      );
      if (rows.length > 0) {
        return rows.map((resource) => ({
          id: `e${i}:${resource}`,
          edge,
          resource,
        }));
      }
    }
    return [{ id: `e${i}`, edge }];
  });
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
    const components = new Set(
      graph.nodes
        .filter((n) => n.kind === "component" || n.kind === "external")
        .map((n) => n.id),
    );
    const drawn = visualEdges(graph);
    const checksOf = new Map<string, AccessCheck[]>(
      [...gateways].map((id) => [id, gatewayChecks(id, graph.edges)]),
    );

    const elkNode = (n: GraphNodeDto): ElkNode => {
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
      if (n.kind === "clients") return { id: n.id, ...CLIENTS_SIZE };
      return {
        id: n.id,
        width: COMPONENT_WIDTH,
        height: componentHeight(n),
        layoutOptions: { "elk.portConstraints": "FIXED_POS" },
        ports: componentPorts(n),
      };
    };

    // External-cloud resources, grouped by cloud, each group one frame.
    const clouds = new Map<string, GraphNodeDto[]>();
    for (const n of graph.nodes) {
      if (n.cloud == null) continue;
      clouds.set(n.cloud, [...(clouds.get(n.cloud) ?? []), n]);
    }
    const children: ElkNode[] = [
      ...graph.nodes.filter((n) => n.cloud == null).map(elkNode),
      ...[...clouds].map(([cloud, members]) => ({
        id: cloudGroupId(cloud),
        layoutOptions: {
          "elk.padding": `[top=${CLOUD_GROUP_HEADER + CLOUD_GROUP_PAD / 2},left=${CLOUD_GROUP_PAD},bottom=${CLOUD_GROUP_PAD},right=${CLOUD_GROUP_PAD}]`,
        },
        children: members.map(elkNode),
      })),
    ];

    const elkGraph: ElkNode = {
      id: "root",
      // With frames, lay out across the hierarchy (edges cross into them) and
      // report every route in root (= flow) coordinates.
      layoutOptions:
        clouds.size > 0
          ? {
              ...LAYOUT_OPTIONS,
              "elk.hierarchyHandling": "INCLUDE_CHILDREN",
              "elk.json.edgeCoords": "ROOT",
            }
          : LAYOUT_OPTIONS,
      children,
      edges: drawn.map(({ id, edge: e, resource }) => {
        const { source, target } = edgeHandles(
          e,
          gateways,
          components,
          resource,
        );
        return {
          id,
          sources: [source ? `${e.from}:${source}` : e.from],
          targets: [target ? `${e.to}:${target}` : e.to],
        };
      }),
    };

    elk
      .layout(elkGraph)
      .then((laidOut) => {
        if (run !== runRef.current) return; // superseded
        // Positions are relative to the parent: the root for most nodes, its
        // frame for an external-cloud resource (as ReactFlow wants a child's).
        const positioned = new Map<string, { x: number; y: number }>();
        const frames = new Map<string, ElkNode>();
        for (const child of laidOut.children ?? []) {
          positioned.set(child.id, { x: child.x ?? 0, y: child.y ?? 0 });
          if (child.children) {
            frames.set(child.id, child);
            for (const member of child.children) {
              positioned.set(member.id, { x: member.x ?? 0, y: member.y ?? 0 });
            }
          }
        }
        // Every edge lives on the root, and with frames ELK reports routes in
        // root coordinates (`edgeCoords: ROOT`), so they are flow coordinates.
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
        // Frames first: ReactFlow needs a parent before its children.
        const frameNodes: Node[] = [...clouds.keys()].map((cloud) => {
          const id = cloudGroupId(cloud);
          const frame = frames.get(id);
          return {
            id,
            type: "cloud",
            position: positioned.get(id) ?? { x: 0, y: 0 },
            width: frame?.width,
            height: frame?.height,
            selectable: false,
            draggable: false,
            focusable: false,
            zIndex: -1,
            data: { cloud } satisfies CloudGroupNodeData,
          };
        });
        const memberNodes: Node[] = graph.nodes.map((n) => {
          const position = positioned.get(n.id) ?? { x: 0, y: 0 };
          const parentId = n.cloud == null ? undefined : cloudGroupId(n.cloud);
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
            parentId,
            data: { node: n, names } satisfies ComponentNodeData,
          };
        });
        const nodes = [...frameNodes, ...memberNodes];
        const edges: Edge<TopologyEdgeData>[] = drawn.map(
          ({ id, edge: e, resource }) => {
            const { source, target } = edgeHandles(
              e,
              gateways,
              components,
              resource,
            );
            return {
              id,
              source: e.from,
              target: e.to,
              sourceHandle: source,
              targetHandle: target,
              type: "elk",
              data: { edge: e, resource, points: routes.get(id) },
            };
          },
        );
        setFlow({ nodes, edges });
      })
      .catch(() => {
        if (run === runRef.current) setFlow({ nodes: [], edges: [] });
      });
  }, [graph]);

  return flow;
}
