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

describe("snapshot file manifest", () => {
  const paths = FIXTURE_PLAN.files.map((f) => f.path);

  test("saves the paths, sorted, without contents, and reads them back", () => {
    const snapshot = createGraphSnapshot(FIXTURE_PLAN.graph, { files: paths });
    expect(snapshot.files).toEqual([...paths].sort());
    const json = stringifyGraphSnapshot(snapshot);
    expect(json).not.toContain(FIXTURE_PLAN.files[0]?.contents ?? "\0");
    expect(roundTrip(JSON.parse(json)).files).toEqual([...paths].sort());
  });

  test("is left out without files, and older snapshots still parse", () => {
    const snapshot = createGraphSnapshot(FIXTURE_PLAN.graph);
    expect("files" in snapshot).toBe(false);
    expect(roundTrip(snapshot).files).toBeUndefined();
  });

  test.each([
    ["not an array", "compose.yaml"],
    ["a non-string", [1]],
    ["an absolute path", ["/etc/passwd"]],
    ["a parent segment", ["modules/../../secret"]],
    ["an empty segment", ["modules//compose.yaml"]],
    ["a duplicate", ["compose.yaml", "compose.yaml"]],
  ])("rejects %s", (_, files) => {
    const snapshot = { ...createGraphSnapshot(FIXTURE_PLAN.graph), files };
    expect(() => roundTrip(snapshot)).toThrow(/graph snapshot:/);
  });
});

describe("snapshot deployment steps", () => {
  test("saves the plan's steps in order and reads them back", () => {
    expect(FIXTURE_PLAN.deploy.length).toBeGreaterThan(0);
    const snapshot = createGraphSnapshot(FIXTURE_PLAN.graph, {
      deploy: FIXTURE_PLAN.deploy,
    });
    expect(snapshot.deploy).toEqual(FIXTURE_PLAN.deploy);
    expect(roundTrip(snapshot).deploy?.map((s) => s.order)).toEqual(
      FIXTURE_PLAN.deploy.map((_, i) => i + 1),
    );
  });

  test("is left out without steps", () => {
    const snapshot = createGraphSnapshot(FIXTURE_PLAN.graph, { deploy: [] });
    expect("deploy" in snapshot).toBe(false);
  });

  test.each([
    ["not an array", { order: 1 }],
    [
      "an unknown phase",
      [{ order: 1, phase: "later", id: "x", title: "X", instructions: "" }],
    ],
    [
      "a missing title",
      [{ order: 1, phase: "start", id: "x", instructions: "" }],
    ],
  ])("rejects %s", (_, deploy) => {
    const snapshot = { ...createGraphSnapshot(FIXTURE_PLAN.graph), deploy };
    expect(() => roundTrip(snapshot)).toThrow(/graph snapshot:/);
  });
});
