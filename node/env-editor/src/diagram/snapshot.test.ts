import { describe, expect, test } from "bun:test";
import { FIXTURE_PLAN } from "@open-lakehouse/stack-wasm/fixtures";
import {
  createGraphSnapshot,
  edgeKey,
  parseGraphSnapshot,
  stringifyGraphSnapshot,
} from "./snapshot";

const roundTrip = (value: unknown) =>
  parseGraphSnapshot(JSON.parse(JSON.stringify(value)));

/** A graph as a snapshot saved before edges carried a `key` or nodes a `model`. */
function legacyGraph() {
  const graph = JSON.parse(JSON.stringify(FIXTURE_PLAN.graph));
  for (const edge of graph.edges) delete edge.key;
  for (const node of graph.nodes) delete node.model;
  return graph;
}

describe("edgeKey", () => {
  test("derives the planner's key for every fixture edge", () => {
    for (const edge of FIXTURE_PLAN.graph.edges) {
      expect(edgeKey(edge)).toBe(edge.key);
    }
  });
});

describe("parseGraphSnapshot", () => {
  test("round-trips a fresh snapshot unchanged", () => {
    const snapshot = createGraphSnapshot(FIXTURE_PLAN.graph, { title: "t" });
    const parsed = roundTrip(JSON.parse(stringifyGraphSnapshot(snapshot)));
    expect(parsed).toEqual(snapshot);
  });

  test("reads a v1 snapshot saved before edge keys, deriving them", () => {
    const parsed = roundTrip({
      format: "trestle-graph",
      version: 1,
      graph: legacyGraph(),
    });
    expect(parsed.graph.edges.map((e) => e.key)).toEqual(
      FIXTURE_PLAN.graph.edges.map((e) => e.key),
    );
    expect(parsed.graph.nodes[0].model).toBeUndefined();
  });

  const base = () => ({
    format: "trestle-graph",
    version: 1,
    graph: JSON.parse(JSON.stringify(FIXTURE_PLAN.graph)),
  });

  test.each([
    [
      "an unsupported version",
      (s: ReturnType<typeof base>) => {
        s.version = 2;
      },
      /unsupported version 2/,
    ],
    [
      "a duplicate node id",
      (s: ReturnType<typeof base>) => {
        s.graph.nodes.push({ ...s.graph.nodes[1] });
      },
      /duplicate node id/,
    ],
    [
      "an edge to a missing node",
      (s: ReturnType<typeof base>) => {
        s.graph.edges[0].to = "nowhere";
      },
      /missing node "nowhere"/,
    ],
    [
      "an unknown edge kind",
      (s: ReturnType<typeof base>) => {
        s.graph.edges[0].kind = "teleports";
      },
      /known `kind`/,
    ],
    [
      "a duplicate edge key",
      (s: ReturnType<typeof base>) => {
        s.graph.edges.push({ ...s.graph.edges[0] });
      },
      /duplicate edge key/,
    ],
    [
      "a node without an id",
      (s: ReturnType<typeof base>) => {
        delete s.graph.nodes[0].id;
      },
      /no string `id`/,
    ],
  ])("rejects %s", (_what, mutate, message) => {
    const snapshot = base();
    mutate(snapshot);
    expect(() => roundTrip(snapshot)).toThrow(message);
  });
});
