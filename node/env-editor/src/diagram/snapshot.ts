// A graph snapshot: a planned environment's functional topology frozen to a
// JSON file (`*.trestle-graph.json`), so a page can embed the diagram exactly as
// it was when exported — with no planner, catalog, or wasm on the page. Later
// planner changes never redraw an existing snapshot.

import type { GraphDto } from "../types";

/** The `format` tag every snapshot carries. */
export const GRAPH_SNAPSHOT_FORMAT = "trestle-graph";

/**
 * The snapshot schema version this build reads and writes. Bump it when a
 * `GraphDto` change would make existing snapshots render wrongly.
 */
export const GRAPH_SNAPSHOT_VERSION = 1;

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
}

/** Freeze a plan's graph into a snapshot. */
export function createGraphSnapshot(
  graph: GraphDto,
  options: { title?: string; generator?: string } = {},
): GraphSnapshot {
  return {
    format: GRAPH_SNAPSHOT_FORMAT,
    version: GRAPH_SNAPSHOT_VERSION,
    generator: options.generator ?? "@open-lakehouse/env-editor",
    ...(options.title ? { title: options.title } : {}),
    graph,
  };
}

/**
 * Validate parsed JSON as a snapshot this build can render. Checks the
 * envelope (format, version, node/edge arrays), not every DTO field; throws
 * an `Error` naming what is wrong.
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
  if (input.version !== GRAPH_SNAPSHOT_VERSION) {
    throw new Error(
      `graph snapshot: unsupported version ${JSON.stringify(input.version)} (this build reads version ${GRAPH_SNAPSHOT_VERSION}); re-export it or upgrade @open-lakehouse/trestle-diagram`,
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
  return input as unknown as GraphSnapshot;
}

/** Serialize a snapshot the way the editor saves it (stable, diff-friendly). */
export function stringifyGraphSnapshot(snapshot: GraphSnapshot): string {
  return `${JSON.stringify(snapshot, null, 2)}\n`;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
