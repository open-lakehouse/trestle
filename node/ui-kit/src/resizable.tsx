import * as React from "react";

import { cn } from "./utils";

export interface ResizeHandleProps
  extends Omit<React.HTMLAttributes<HTMLDivElement>, "onResize"> {
  /** `vertical` sits between side-by-side panes (drag left/right);
   *  `horizontal` between stacked ones (drag up/down). */
  orientation: "vertical" | "horizontal";
  /** Called with how far the pointer moved since the last call, in pixels:
   *  positive is right / down. */
  onResize: (delta: number) => void;
  /** The size of the pane it resizes, in pixels (its `aria-valuenow`). */
  size: number;
}

/**
 * A draggable divider between two panes. It reports pointer movement; the
 * owner keeps the pane's size (see {@link usePaneSize}). Arrow keys nudge it
 * when focused.
 */
function ResizeHandle({
  orientation,
  onResize,
  size,
  className,
  ...props
}: ResizeHandleProps) {
  const last = React.useRef<number | null>(null);
  const vertical = orientation === "vertical";
  return (
    // biome-ignore lint/a11y/useSemanticElements: a draggable splitter, not a rule
    <div
      role="separator"
      aria-orientation={vertical ? "vertical" : "horizontal"}
      aria-valuenow={size}
      tabIndex={0}
      className={cn(
        "relative shrink-0 bg-border transition-colors hover:bg-primary/60 focus-visible:bg-primary focus-visible:outline-none",
        vertical ? "w-px cursor-col-resize" : "h-px cursor-row-resize",
        // A wider invisible grab area around the 1px line.
        vertical
          ? "after:absolute after:inset-y-0 after:-left-1 after:-right-1"
          : "after:absolute after:inset-x-0 after:-top-1 after:-bottom-1",
        className,
      )}
      onPointerDown={(e) => {
        e.currentTarget.setPointerCapture(e.pointerId);
        last.current = vertical ? e.clientX : e.clientY;
      }}
      onPointerMove={(e) => {
        if (last.current === null) return;
        const at = vertical ? e.clientX : e.clientY;
        onResize(at - last.current);
        last.current = at;
      }}
      onPointerUp={(e) => {
        e.currentTarget.releasePointerCapture(e.pointerId);
        last.current = null;
      }}
      onKeyDown={(e) => {
        const step = e.shiftKey ? 48 : 12;
        const back = vertical ? "ArrowLeft" : "ArrowUp";
        const forward = vertical ? "ArrowRight" : "ArrowDown";
        if (e.key === back) onResize(-step);
        else if (e.key === forward) onResize(step);
        else return;
        e.preventDefault();
      }}
      {...props}
    />
  );
}

/**
 * A pane size in pixels, clamped to `[min, max]`, with a `resize(delta)` for a
 * {@link ResizeHandle}. `invert` flips the delta, for a pane on the far side of
 * its handle (a right sidebar, a bottom panel).
 */
function usePaneSize(
  initial: number,
  { min, max, invert = false }: { min: number; max: number; invert?: boolean },
): [number, (delta: number) => void] {
  const [size, setSize] = React.useState(initial);
  const resize = React.useCallback(
    (delta: number) =>
      setSize((s) =>
        Math.min(max, Math.max(min, s + (invert ? -delta : delta))),
      ),
    [min, max, invert],
  );
  return [size, resize];
}

export { ResizeHandle, usePaneSize };
