// Lateral (service-to-service) edges: a component calling a peer's API, e.g. an
// experiment tracker whose model registry is the data catalog. ELK's layered
// algorithm puts the two ends of every edge in different layers, so a chain of
// peers the gateway exposes side by side would become a staircase of columns.
// The layout therefore leaves these edges out of ELK, which keeps the peers in
// one column, and routes them here, vertically through that column:
//
// - neighbours in the column connect bottom → top (or top → bottom), straight
//   through the gap between them;
// - peers with other cards between them connect through a lane in the
//   column's east gutter, closer to the column than ELK's own channel routes
//   (which keep `edgeNodeBetweenLayers` away from it).
//
// An edge that fits neither (its ends landed in different columns, a route
// would cross a card, or the gutter is full) is rejected, and the layout hands
// it back to ELK as an ordinary edge.

import type { TopologyEdgeDto } from "../types";
import type { Point } from "./edges/ElkEdge";

/** A laid-out card (or frame) in flow coordinates. */
export interface Box {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** A lateral edge to route: `from` calls `to`. */
export interface LateralEdge {
  id: string;
  from: string;
  to: string;
}

/** A routed lateral edge: its points and the handles it attaches to. */
export interface LateralRoute {
  points: Point[];
  source: string;
  target: string;
}

/** The handles lateral routes attach to on a component card. */
export const TOP_IN = "top-in";
export const TOP_OUT = "top-out";
export const BOTTOM_IN = "bottom-in";
export const BOTTOM_OUT = "bottom-out";
export const EAST_IN = "east-in";
export const EAST_OUT = "east-out";

/** Horizontal distance between parallel routes through one gap. */
const GAP_STEP = 24;
/** The first gutter lane's distance from the column, and between lanes. Every
 *  lane stays inside ELK's `edgeNodeBetweenLayers` margin. */
const LANE_START = 10;
const LANE_STEP = 8;
const LANES = 3;
/** Vertical clearance between two routes sharing a gutter lane. */
const LANE_CLEARANCE = 8;
/** How far below a card's top a gutter route leaves or enters it: under the
 *  header's generic out port, so it doesn't run along ELK's edges from it. */
export const GUTTER_INSET = 42;

/** Whether `edge` is lateral: a component using a peer's API (no database or
 *  bucket row to land on), neither of them inside an external-cloud frame. */
export function isLateral(
  edge: TopologyEdgeDto,
  resource: string | undefined,
  components: Set<string>,
  framed: Set<string>,
): boolean {
  return (
    edge.kind === "uses" &&
    resource === undefined &&
    edge.from !== edge.to &&
    components.has(edge.from) &&
    components.has(edge.to) &&
    !framed.has(edge.from) &&
    !framed.has(edge.to)
  );
}

const right = (b: Box) => b.x + b.width;
const bottom = (b: Box) => b.y + b.height;
const centerX = (b: Box) => b.x + b.width / 2;

/** Whether `a` and `b` are cards of one column: the same width, centred on
 *  the same x. */
const sameColumn = (a: Box, b: Box) =>
  Math.abs(centerX(a) - centerX(b)) < 1 && Math.abs(a.width - b.width) < 1;

/** Whether the axis-parallel segment `p`–`q` crosses box `b`'s interior. */
function hits(p: Point, q: Point, b: Box): boolean {
  const [x0, x1] = [Math.min(p.x, q.x), Math.max(p.x, q.x)];
  const [y0, y1] = [Math.min(p.y, q.y), Math.max(p.y, q.y)];
  return x0 < right(b) && x1 > b.x && y0 < bottom(b) && y1 > b.y;
}

/** Whether a route through `points` crosses any box other than its ends. */
function blocked(points: Point[], boxes: Map<string, Box>, ends: string[]) {
  for (const [id, b] of boxes) {
    if (ends.includes(id)) continue;
    for (let i = 1; i < points.length; i++) {
      if (hits(points[i - 1], points[i], b)) return true;
    }
  }
  return false;
}

/**
 * Route `edges` through the columns of the laid-out `boxes` (every card and
 * frame, keyed by id). Returns the routes found and the ids of edges that
 * need ELK instead.
 */
export function routeLateral(
  edges: LateralEdge[],
  boxes: Map<string, Box>,
): { routes: Map<string, LateralRoute>; rejected: string[] } {
  const routes = new Map<string, LateralRoute>();
  const rejected: string[] = [];
  // Edges between neighbours, grouped by the gap they cross.
  const gaps = new Map<string, LateralEdge[]>();
  const gutter: LateralEdge[] = [];

  for (const e of edges) {
    const [s, t] = [boxes.get(e.from), boxes.get(e.to)];
    if (!s || !t || e.from === e.to || !sameColumn(s, t)) {
      rejected.push(e.id);
      continue;
    }
    const [upper, lower] = s.y < t.y ? [s, t] : [t, s];
    const between = [...boxes].some(
      ([id, b]) =>
        id !== e.from &&
        id !== e.to &&
        b.x < right(s) &&
        right(b) > s.x &&
        b.y < lower.y &&
        bottom(b) > bottom(upper),
    );
    if (between) {
      gutter.push(e);
    } else {
      const key = [e.from, e.to].sort().join("\u0000");
      gaps.set(key, [...(gaps.get(key) ?? []), e]);
    }
  }

  for (const group of gaps.values()) {
    group.forEach((e, i) => {
      const [s, t] = [boxes.get(e.from) as Box, boxes.get(e.to) as Box];
      const x = centerX(s) + (i - (group.length - 1) / 2) * GAP_STEP;
      const down = s.y < t.y;
      const points = down
        ? [
            { x, y: bottom(s) },
            { x, y: t.y },
          ]
        : [
            { x, y: s.y },
            { x, y: bottom(t) },
          ];
      if (blocked(points, boxes, [e.from, e.to])) {
        rejected.push(e.id);
      } else {
        routes.set(e.id, {
          points,
          source: down ? BOTTOM_OUT : TOP_OUT,
          target: down ? TOP_IN : BOTTOM_IN,
        });
      }
    });
  }

  // Gutter lanes per column, shortest spans nearest the column. A lane holds
  // routes whose vertical spans don't overlap.
  const lanes = new Map<number, [number, number][][]>();
  const span = (e: LateralEdge): [number, number] => {
    const [s, t] = [boxes.get(e.from) as Box, boxes.get(e.to) as Box];
    const [a, b] = [s.y + GUTTER_INSET, t.y + GUTTER_INSET];
    return [Math.min(a, b), Math.max(a, b)];
  };
  gutter.sort((a, b) => {
    const [sa, sb] = [span(a), span(b)];
    return sa[1] - sa[0] - (sb[1] - sb[0]) || sa[0] - sb[0];
  });
  for (const e of gutter) {
    const [s, t] = [boxes.get(e.from) as Box, boxes.get(e.to) as Box];
    const column = lanes.get(centerX(s)) ?? [];
    lanes.set(centerX(s), column);
    const [y0, y1] = span(e);
    let routed = false;
    for (let k = 0; k < LANES && !routed; k++) {
      const lane = column[k] ?? [];
      if (
        lane.some(
          ([a, b]) => y0 < b + LANE_CLEARANCE && y1 > a - LANE_CLEARANCE,
        )
      )
        continue;
      const x = right(s) + LANE_START + k * LANE_STEP;
      const points = [
        { x: right(s), y: s.y + GUTTER_INSET },
        { x, y: s.y + GUTTER_INSET },
        { x, y: t.y + GUTTER_INSET },
        { x: right(t), y: t.y + GUTTER_INSET },
      ];
      if (blocked(points, boxes, [e.from, e.to])) continue;
      column[k] = [...lane, [y0, y1]];
      routes.set(e.id, { points, source: EAST_OUT, target: EAST_IN });
      routed = true;
    }
    if (!routed) rejected.push(e.id);
  }

  return { routes, rejected };
}

/** The most partners `arrangeColumns` permutes as one run; a larger group
 *  keeps ELK's order among its members. */
const MAX_PERMUTED = 8;

function permutations<T>(items: T[]): T[][] {
  if (items.length <= 1) return [items];
  return items.flatMap((item, i) =>
    permutations([...items.slice(0, i), ...items.slice(i + 1)]).map((rest) => [
      item,
      ...rest,
    ]),
  );
}

/** How far apart `edges`' ends are in `run` (0 when every pair is adjacent),
 *  then how far it moves them from their order in `from`. */
function runCost(run: string[], edges: LateralEdge[], from: string[]) {
  const at = new Map(run.map((id, i) => [id, i]));
  let apart = 0;
  for (const e of edges) {
    const [a, b] = [at.get(e.from), at.get(e.to)];
    if (a !== undefined && b !== undefined) apart += Math.abs(a - b) - 1;
  }
  const moved = run.reduce((m, id, i) => m + Math.abs(i - from.indexOf(id)), 0);
  return [apart, moved] as const;
}

/**
 * Reorder each column of the laid-out `boxes` so lateral partners sit next to
 * each other: every group of partners in a column becomes one run, in the
 * order that keeps the most pairs adjacent, placed where it moves the other
 * cards least. Returns the new top-to-bottom order of each column it changed.
 *
 * ELK's crossing minimization doesn't know about the omitted lateral edges, so
 * it often splits partners; the layout feeds this order back to ELK as
 * positions for an interactive pass.
 */
export function arrangeColumns(
  boxes: Map<string, Box>,
  edges: LateralEdge[],
): string[][] {
  const columns = new Map<string, string[]>();
  for (const [id, b] of boxes) {
    const key = `${centerX(b)}:${b.width}`;
    columns.set(key, [...(columns.get(key) ?? []), id]);
  }
  const changed: string[][] = [];
  for (const ids of columns.values()) {
    const original = ids.sort(
      (a, b) => (boxes.get(a) as Box).y - (boxes.get(b) as Box).y,
    );
    const inColumn = edges.filter(
      (e) =>
        e.from !== e.to && original.includes(e.from) && original.includes(e.to),
    );
    if (inColumn.length === 0) continue;

    // Groups of partners: connected components of the column's lateral edges.
    const group = new Map<string, string>(original.map((id) => [id, id]));
    const root = (id: string): string => {
      const parent = group.get(id) as string;
      return parent === id ? id : root(parent);
    };
    for (const e of inColumn) group.set(root(e.from), root(e.to));
    const groups = new Map<string, string[]>();
    for (const id of original) {
      if (!inColumn.some((e) => e.from === id || e.to === id)) continue;
      groups.set(root(id), [...(groups.get(root(id)) ?? []), id]);
    }

    let order = original;
    for (const members of groups.values()) {
      const runs =
        members.length <= MAX_PERMUTED ? permutations(members) : [members];
      let run = members;
      let best = runCost(members, inColumn, members);
      for (const candidate of runs) {
        const c = runCost(candidate, inColumn, members);
        if (c[0] < best[0] || (c[0] === best[0] && c[1] < best[1])) {
          run = candidate;
          best = c;
        }
      }
      const rest = order.filter((id) => !members.includes(id));
      let placed = order;
      let least = Number.POSITIVE_INFINITY;
      for (let i = 0; i <= rest.length; i++) {
        const next = [...rest.slice(0, i), ...run, ...rest.slice(i)];
        const moved = next.reduce(
          (m, id, j) => m + Math.abs(j - original.indexOf(id)),
          0,
        );
        if (moved < least) {
          least = moved;
          placed = next;
        }
      }
      order = placed;
    }
    if (order.some((id, i) => id !== original[i])) changed.push(order);
  }
  return changed;
}
