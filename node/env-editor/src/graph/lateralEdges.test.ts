import { describe, expect, test } from "bun:test";
import type { TopologyEdgeDto } from "../types";
import {
  arrangeColumns,
  BOTTOM_IN,
  BOTTOM_OUT,
  type Box,
  EAST_IN,
  EAST_OUT,
  GUTTER_INSET,
  isLateral,
  routeLateral,
  TOP_IN,
  TOP_OUT,
} from "./lateralEdges";

const card = (x: number, y: number, height = 52): Box => ({
  x,
  y,
  width: 240,
  height,
});

const uses = (from: string, to: string): TopologyEdgeDto => ({
  key: `uses:${from}→${to}`,
  kind: "uses",
  from,
  to,
  role: "data_catalog",
  protocol: "HTTP",
  resources: [],
});

describe("isLateral", () => {
  const components = new Set(["mlflow", "unity-catalog", "aws-s3"]);
  const framed = new Set(["aws-s3"]);

  test("a component using a peer's API", () => {
    expect(
      isLateral(uses("mlflow", "unity-catalog"), undefined, components, framed),
    ).toBe(true);
  });

  test("not a use landing on a resource row", () => {
    expect(
      isLateral(uses("mlflow", "unity-catalog"), "mlflow", components, framed),
    ).toBe(false);
  });

  test("not the gateway's edges, nor a framed resource", () => {
    const route: TopologyEdgeDto = {
      key: "route",
      kind: "route",
      from: "envoy",
      to: "mlflow",
      host_port: 9080,
      gated: false,
    };
    expect(isLateral(route, undefined, components, framed)).toBe(false);
    expect(
      isLateral(uses("mlflow", "aws-s3"), undefined, components, framed),
    ).toBe(false);
  });
});

describe("routeLateral", () => {
  test("neighbours connect through the gap between them", () => {
    const boxes = new Map([
      ["uc", card(100, 0)],
      ["mlflow", card(100, 100)],
    ]);
    const { routes, rejected } = routeLateral(
      [
        { id: "up", from: "mlflow", to: "uc" },
        { id: "down", from: "uc", to: "mlflow" },
      ],
      boxes,
    );
    expect(rejected).toEqual([]);
    const up = routes.get("up");
    expect(up?.source).toBe(TOP_OUT);
    expect(up?.target).toBe(BOTTOM_IN);
    expect(up?.points.map((p) => p.y)).toEqual([100, 52]);
    const down = routes.get("down");
    expect(down?.source).toBe(BOTTOM_OUT);
    expect(down?.target).toBe(TOP_IN);
    expect(down?.points.map((p) => p.y)).toEqual([52, 100]);
    // Two routes through one gap run side by side, around the centre.
    const xs = [up?.points[0].x, down?.points[0].x];
    expect(new Set(xs).size).toBe(2);
    expect(((xs[0] ?? 0) + (xs[1] ?? 0)) / 2).toBe(220);
  });

  test("peers with a card between them use the east gutter", () => {
    const boxes = new Map([
      ["uc", card(100, 0)],
      ["mlflow", card(100, 100)],
      ["graph", card(100, 200)],
    ]);
    const { routes, rejected } = routeLateral(
      [{ id: "e", from: "graph", to: "uc" }],
      boxes,
    );
    expect(rejected).toEqual([]);
    const route = routes.get("e");
    expect(route?.source).toBe(EAST_OUT);
    expect(route?.target).toBe(EAST_IN);
    expect(route?.points).toEqual([
      { x: 340, y: 200 + GUTTER_INSET },
      { x: 350, y: 200 + GUTTER_INSET },
      { x: 350, y: GUTTER_INSET },
      { x: 340, y: GUTTER_INSET },
    ]);
  });

  test("overlapping gutter routes take separate lanes", () => {
    const boxes = new Map(
      ["a", "b", "c", "d"].map((id, i) => [id, card(0, i * 100)]),
    );
    const { routes, rejected } = routeLateral(
      [
        { id: "ac", from: "a", to: "c" },
        { id: "bd", from: "b", to: "d" },
      ],
      boxes,
    );
    expect(rejected).toEqual([]);
    const lane = (id: string) => routes.get(id)?.points[1].x;
    expect(lane("ac")).not.toBe(lane("bd"));
  });

  test("ends in different columns are left to ELK", () => {
    const boxes = new Map([
      ["uc", card(100, 0)],
      ["mlflow", card(460, 0)],
    ]);
    const { routes, rejected } = routeLateral(
      [{ id: "e", from: "mlflow", to: "uc" }],
      boxes,
    );
    expect(routes.size).toBe(0);
    expect(rejected).toEqual(["e"]);
  });

  test("a route that would cross a box is left to ELK", () => {
    // A frame wider than the column, sitting in the gap.
    const boxes = new Map([
      ["uc", card(100, 0)],
      ["mlflow", card(100, 200)],
      ["@cloud:aws", { x: 0, y: 80, width: 600, height: 60 }],
    ]);
    const { rejected } = routeLateral(
      [{ id: "e", from: "mlflow", to: "uc" }],
      boxes,
    );
    expect(rejected).toEqual(["e"]);
  });
});

describe("arrangeColumns", () => {
  /** One column of cards, top to bottom in `ids` order. */
  const column = (ids: string[], x = 0) =>
    new Map(ids.map((id, i) => [id, card(x, i * 100)]));
  const edges = (pairs: [string, string][]) =>
    pairs.map(([from, to], i) => ({ id: `e${i}`, from, to }));

  test("regroups split partners into one adjacent run", () => {
    // UC is used by MLflow and PuppyGraph, MLflow by Omnigent; ELK split them.
    const boxes = column(["uc", "mlflow", "omnigent", "rivers", "graph"]);
    const [order, ...rest] = arrangeColumns(
      boxes,
      edges([
        ["graph", "uc"],
        ["mlflow", "uc"],
        ["omnigent", "mlflow"],
      ]),
    );
    expect(rest).toEqual([]);
    const at = (id: string) => order.indexOf(id);
    expect(Math.abs(at("graph") - at("uc"))).toBe(1);
    expect(Math.abs(at("mlflow") - at("uc"))).toBe(1);
    expect(Math.abs(at("omnigent") - at("mlflow"))).toBe(1);
    expect(order).toContain("rivers");
  });

  test("moves the other cards as little as possible", () => {
    const boxes = column(["a", "x", "y", "b"]);
    expect(arrangeColumns(boxes, edges([["a", "b"]]))).toEqual([
      ["a", "b", "x", "y"],
    ]);
  });

  test("leaves a column whose partners are already adjacent", () => {
    const boxes = column(["uc", "mlflow", "omnigent"]);
    const lateral = edges([
      ["mlflow", "uc"],
      ["omnigent", "mlflow"],
    ]);
    expect(arrangeColumns(boxes, lateral)).toEqual([]);
  });

  test("ignores partners in different columns", () => {
    const boxes = new Map([...column(["a", "x"]), ...column(["b"], 400)]);
    expect(arrangeColumns(boxes, edges([["a", "b"]]))).toEqual([]);
  });
});
