// A graph snapshot: a planned environment's functional topology frozen to a
// JSON file (`*.trestle-graph.json`), so a page can embed the diagram exactly as
// it was when exported — with no planner, catalog, or wasm on the page. Later
// planner changes never redraw an existing snapshot.

import type { DeployStepDto, GraphDto, TopologyEdgeDto } from "../types";

/** The `format` tag every snapshot carries. */
export const GRAPH_SNAPSHOT_FORMAT = "trestle-graph";

/**
 * The snapshot schema version this build reads and writes. Bump it when a
 * `GraphDto` change would make existing snapshots render wrongly.
 */
export const GRAPH_SNAPSHOT_VERSION = 1;

/** Every snapshot version this build can read. Version 1 snapshots saved
 *  before edges carried a `key` get one derived on read (see {@link edgeKey}). */
export const SUPPORTED_GRAPH_SNAPSHOT_VERSIONS: readonly number[] = [1];

/** The file extension snapshots are saved under. */
export const GRAPH_SNAPSHOT_EXTENSION = ".trestle-graph.json";

/** A frozen environment diagram. */
export interface GraphSnapshot {
  format: typeof GRAPH_SNAPSHOT_FORMAT;
  version: typeof GRAPH_SNAPSHOT_VERSION;
  /** What produced the snapshot (e.g. `@open-lakehouse/env-editor`). */
  generator?: string;
  /** A human title for the environment, if the author gave one. */
  title?: string;
  /** The plan's functional topology, as the planner projected it. */
  graph: GraphDto;
  /**
   * The rendered environment's viewable files: paths relative to its root,
   * without contents. An embed reads them from the render shipped next to the
   * snapshot (`EnvironmentDiagram`'s `readFile`) and offers only these, so a
   * secret in the render is never shown. Absent on older snapshots.
   */
  files?: string[];
  /**
   * The environment's deployment steps, in order: what an operator does beyond
   * starting the stack (create hosted resources, review accounts). A step's
   * `script` is a path among `files`. For tools that walk someone through, or
   * automate, the deployment. Absent on older snapshots.
   */
  deploy?: DeployStepDto[];
  /**
   * The graph as a Mermaid flowchart (the planner's `mermaid`), for pasting
   * into Markdown. Absent on older snapshots.
   */
  mermaid?: string;
}

/** Freeze a plan's graph (and its file manifest and deployment steps, if
 *  given) into a snapshot. */
export function createGraphSnapshot(
  graph: GraphDto,
  options: {
    title?: string;
    generator?: string;
    /** The plan's viewable file paths (`plan.files`' paths, no contents). */
    files?: readonly string[];
    /** The plan's deployment steps (`plan.deploy`). */
    deploy?: readonly DeployStepDto[];
    /** The plan's Mermaid diagram (`plan.mermaid`). */
    mermaid?: string;
  } = {},
): GraphSnapshot {
  return {
    format: GRAPH_SNAPSHOT_FORMAT,
    version: GRAPH_SNAPSHOT_VERSION,
    generator: options.generator ?? "@open-lakehouse/env-editor",
    ...(options.title ? { title: options.title } : {}),
    graph,
    ...(options.files?.length ? { files: [...options.files].sort() } : {}),
    ...(options.deploy?.length ? { deploy: [...options.deploy] } : {}),
    ...(options.mermaid ? { mermaid: options.mermaid } : {}),
  };
}

/**
 * Validate parsed JSON as a snapshot this build can render: the envelope
 * (format, a supported version, node/edge arrays) and the graph's integrity
 * (unique node ids, edges of a known kind between existing nodes, unique edge
 * keys), a file manifest's paths (unique, relative, within the render), and
 * that a Mermaid diagram is a string.
 * Other DTO fields aren't checked. Throws an `Error` naming what is
 * wrong; an edge without a `key` gets the one the planner would assign.
 */
export function parseGraphSnapshot(input: unknown): GraphSnapshot {
  if (!isRecord(input)) {
    throw new Error("graph snapshot: expected a JSON object");
  }
  if (input.format !== GRAPH_SNAPSHOT_FORMAT) {
    throw new Error(
      `graph snapshot: expected format "${GRAPH_SNAPSHOT_FORMAT}", got ${JSON.stringify(input.format)}`,
    );
  }
  if (
    typeof input.version !== "number" ||
    !SUPPORTED_GRAPH_SNAPSHOT_VERSIONS.includes(input.version)
  ) {
    throw new Error(
      `graph snapshot: unsupported version ${JSON.stringify(input.version)} (this build reads version ${SUPPORTED_GRAPH_SNAPSHOT_VERSIONS.join(", ")}); re-export it or upgrade @open-lakehouse/trestle-diagram`,
    );
  }
  const { graph } = input;
  if (
    !isRecord(graph) ||
    !Array.isArray(graph.nodes) ||
    !Array.isArray(graph.edges)
  ) {
    throw new Error(
      "graph snapshot: `graph` must have `nodes` and `edges` arrays",
    );
  }

  const ids = new Set<string>();
  for (const [i, node] of graph.nodes.entries()) {
    if (!isRecord(node) || typeof node.id !== "string") {
      throw new Error(`graph snapshot: node ${i} has no string \`id\``);
    }
    if (ids.has(node.id)) {
      throw new Error(`graph snapshot: duplicate node id "${node.id}"`);
    }
    ids.add(node.id);
  }

  const keys = new Set<string>();
  const edges = graph.edges.map((edge: unknown, i: number) => {
    if (
      !isRecord(edge) ||
      typeof edge.kind !== "string" ||
      !EDGE_KINDS.has(edge.kind) ||
      typeof edge.from !== "string" ||
      typeof edge.to !== "string"
    ) {
      throw new Error(
        `graph snapshot: edge ${i} needs a known \`kind\` and string \`from\`/\`to\``,
      );
    }
    for (const end of [edge.from, edge.to]) {
      if (!ids.has(end)) {
        throw new Error(
          `graph snapshot: edge ${i} (${edge.kind}) references missing node "${end}"`,
        );
      }
    }
    const keyed = (
      typeof edge.key === "string"
        ? edge
        : { ...edge, key: edgeKey(edge as unknown as TopologyEdgeDto) }
    ) as TopologyEdgeDto;
    if (keys.has(keyed.key)) {
      throw new Error(`graph snapshot: duplicate edge key "${keyed.key}"`);
    }
    keys.add(keyed.key);
    return keyed;
  });

  if (input.files !== undefined) checkManifest(input.files);
  if (input.deploy !== undefined) checkDeploy(input.deploy);
  if (input.mermaid !== undefined && typeof input.mermaid !== "string") {
    throw new Error("graph snapshot: `mermaid` must be a string");
  }

  return {
    ...input,
    graph: { ...graph, edges },
  } as unknown as GraphSnapshot;
}

/** A manifest lists unique relative paths that stay inside the render. */
function checkManifest(files: unknown): void {
  if (!Array.isArray(files)) {
    throw new Error("graph snapshot: `files` must be an array of paths");
  }
  const seen = new Set<string>();
  for (const [i, path] of files.entries()) {
    if (
      typeof path !== "string" ||
      path === "" ||
      path.startsWith("/") ||
      path.includes("\\") ||
      path
        .split("/")
        .some((part) => part === "" || part === "." || part === "..")
    ) {
      throw new Error(
        `graph snapshot: file ${i} must be a relative path inside the render, got ${JSON.stringify(path)}`,
      );
    }
    if (seen.has(path)) {
      throw new Error(`graph snapshot: duplicate file "${path}"`);
    }
    seen.add(path);
  }
}

/** Deployment steps carry an order, a phase, an id, a title and instructions. */
function checkDeploy(deploy: unknown): void {
  if (!Array.isArray(deploy)) {
    throw new Error("graph snapshot: `deploy` must be an array of steps");
  }
  for (const [i, step] of deploy.entries()) {
    if (
      !isRecord(step) ||
      typeof step.order !== "number" ||
      !DEPLOY_PHASES.has(step.phase as string) ||
      typeof step.id !== "string" ||
      typeof step.title !== "string" ||
      typeof step.instructions !== "string"
    ) {
      throw new Error(
        `graph snapshot: deploy step ${i} needs an \`order\`, a known \`phase\`, and string \`id\`/\`title\`/\`instructions\``,
      );
    }
  }
}

const DEPLOY_PHASES: ReadonlySet<string> = new Set<DeployStepDto["phase"]>([
  "provision",
  "configure",
  "start",
  "verify",
]);

/**
 * The key the planner assigns `edge` (Rust `TopologyEdgeDto::derive_key`):
 * `kind:from→to:qualifier`, where the qualifier is the capability for a
 * `uses` edge and the surface's host port for the gateway's edges.
 */
export function edgeKey(edge: TopologyEdgeDto): string {
  const qualifier = edge.kind === "uses" ? edge.role : String(edge.host_port);
  return `${edge.kind}:${edge.from}→${edge.to}:${qualifier}`;
}

const EDGE_KINDS: ReadonlySet<string> = new Set<TopologyEdgeDto["kind"]>([
  "access",
  "route",
  "authenticates",
  "authorizes",
  "uses",
]);

/** Serialize a snapshot the way the editor saves it (stable, diff-friendly). */
export function stringifyGraphSnapshot(snapshot: GraphSnapshot): string {
  return `${JSON.stringify(snapshot, null, 2)}\n`;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
