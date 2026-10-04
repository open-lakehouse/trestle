import { BaseEdge, type EdgeProps } from "@xyflow/react";

/** A point on an ELK-routed edge, in flow coordinates. */
export interface Point {
  x: number;
  y: number;
}

/** Corner radius for the rounded orthogonal bends. */
const RADIUS = 8;

/**
 * An SVG path through orthogonal `points`, with each bend rounded by a
 * quadratic curve (clamped so short segments don't overshoot).
 */
export function roundedPath(points: Point[]): string {
  if (points.length === 0) return "";
  let d = `M ${points[0].x} ${points[0].y}`;
  for (let i = 1; i < points.length - 1; i++) {
    const [prev, cur, next] = [points[i - 1], points[i], points[i + 1]];
    const inLen = Math.hypot(cur.x - prev.x, cur.y - prev.y);
    const outLen = Math.hypot(next.x - cur.x, next.y - cur.y);
    const r = Math.min(RADIUS, inLen / 2, outLen / 2);
    const a = {
      x: cur.x - ((cur.x - prev.x) / (inLen || 1)) * r,
      y: cur.y - ((cur.y - prev.y) / (inLen || 1)) * r,
    };
    const b = {
      x: cur.x + ((next.x - cur.x) / (outLen || 1)) * r,
      y: cur.y + ((next.y - cur.y) / (outLen || 1)) * r,
    };
    d += ` L ${a.x} ${a.y} Q ${cur.x} ${cur.y} ${b.x} ${b.y}`;
  }
  const last = points[points.length - 1];
  return `${d} L ${last.x} ${last.y}`;
}

/** The midpoint of the longest segment: where a label reads best. */
export function labelPoint(points: Point[]): Point {
  let best = { x: 0, y: 0 };
  let bestLen = -1;
  for (let i = 1; i < points.length; i++) {
    const [a, b] = [points[i - 1], points[i]];
    const len = Math.hypot(b.x - a.x, b.y - a.y);
    if (len > bestLen) {
      bestLen = len;
      best = { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
    }
  }
  return best;
}

/**
 * Draws the route ELK computed for this edge (`data.points`), so edges avoid
 * nodes and keep the layout's channels instead of being re-routed by
 * ReactFlow. Falls back to a straight line between the handles when no route
 * is available.
 */
export function ElkEdge({
  sourceX,
  sourceY,
  targetX,
  targetY,
  data,
  markerEnd,
  style,
  label,
  labelStyle,
  labelBgStyle,
  labelBgPadding,
  interactionWidth,
}: EdgeProps) {
  const routed = (data as { points?: Point[] } | undefined)?.points;
  const points =
    routed && routed.length >= 2
      ? routed
      : [
          { x: sourceX, y: sourceY },
          { x: targetX, y: targetY },
        ];
  const at = labelPoint(points);
  return (
    <BaseEdge
      path={roundedPath(points)}
      markerEnd={markerEnd}
      style={style}
      label={label}
      labelX={at.x}
      labelY={at.y}
      labelStyle={labelStyle}
      labelShowBg
      labelBgStyle={labelBgStyle}
      labelBgPadding={labelBgPadding}
      interactionWidth={interactionWidth}
    />
  );
}
