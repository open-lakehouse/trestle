// Export the rendered diagram as an image: frame the laid-out nodes, then
// rasterize (PNG) or serialize (SVG) React Flow's viewport layer. This is React
// Flow's documented download-image recipe, framed to the node bounds so the
// image is the whole diagram regardless of the current pan/zoom. Panels (the
// legend, toolbars) and controls sit outside the viewport layer, so they are
// left out.

import { useReactFlow, useStore, useStoreApi } from "@xyflow/react";
import { toPng, toSvg } from "html-to-image";
import { useCallback } from "react";

export type DiagramImageFormat = "svg" | "png";

export interface DiagramImageOptions {
  format: DiagramImageFormat;
  /** Margin around the diagram, in CSS pixels. Defaults to 24. */
  padding?: number;
  /** PNG resolution multiplier. Defaults to 2 (crisp on high-DPI screens). */
  pixelRatio?: number;
  /**
   * The image background: a CSS color, or `"transparent"`. Defaults to the
   * canvas's own background, so the export matches what is on screen
   * (including the active light/dark theme).
   */
  background?: string;
}

/**
 * Returns a function that renders the surrounding React Flow diagram to an
 * image data URL. Call it from inside the flow (e.g. a `<Panel>` passed as a
 * child of `MarkitectureCanvas`).
 *
 * The SVG wraps the diagram's HTML in a `foreignObject`: it scales losslessly
 * in browsers, but tools that only read plain vector SVG (some feed readers,
 * social-card scrapers) want the PNG.
 */
export function useDiagramImage(): (
  options: DiagramImageOptions,
) => Promise<string> {
  const { getNodes, getNodesBounds } = useReactFlow();
  const store = useStoreApi();

  return useCallback(
    async ({ format, padding = 24, pixelRatio = 2, background }) => {
      const domNode = store.getState().domNode;
      const viewport = domNode?.querySelector<HTMLElement>(
        ".react-flow__viewport",
      );
      if (!domNode || !viewport) throw new Error("the diagram is not mounted");
      const nodes = getNodes();
      if (nodes.length === 0) throw new Error("the diagram has no components");

      const bounds = getNodesBounds(nodes);
      const width = Math.ceil(bounds.width + 2 * padding);
      const height = Math.ceil(bounds.height + 2 * padding);
      const options = {
        width,
        height,
        backgroundColor:
          background === "transparent"
            ? undefined
            : (background ?? canvasBackground(domNode)),
        // Re-frame the viewport layer onto the node bounds at 1:1 scale.
        style: {
          width: `${width}px`,
          height: `${height}px`,
          transform: `translate(${padding - bounds.x}px, ${padding - bounds.y}px) scale(1)`,
        },
      };
      // The edges are SVG, whose styles html-to-image copies verbatim, and they
      // paint with `hsl(var(--token))`. The capture holds only the viewport
      // layer, so it would lose the tokens its ancestors define: pin their
      // resolved values on the viewport for the duration of the capture.
      const release = pinCustomProperties(viewport);
      try {
        return format === "png"
          ? await toPng(viewport, { ...options, pixelRatio })
          : await toSvg(viewport, options);
      } finally {
        release();
      }
    },
    [getNodes, getNodesBounds, store],
  );
}

/** Tokens the canvas paints SVG with, in case the browser does not list
 *  custom properties when iterating a computed style. */
const CANVAS_TOKENS = [
  "--background",
  "--border",
  "--card",
  "--muted-foreground",
  "--primary",
  "--status-blocked",
];

/**
 * Set every custom property `el` inherits as an inline style on it, and
 * return a function that removes exactly those again.
 */
function pinCustomProperties(el: HTMLElement): () => void {
  const computed = getComputedStyle(el);
  const names = new Set(CANVAS_TOKENS);
  for (const name of Array.from(computed)) {
    if (name.startsWith("--")) names.add(name);
  }
  const pinned: string[] = [];
  for (const name of names) {
    const value = computed.getPropertyValue(name).trim();
    if (value && el.style.getPropertyValue(name) === "") {
      el.style.setProperty(name, value);
      pinned.push(name);
    }
  }
  return () => {
    for (const name of pinned) el.style.removeProperty(name);
  };
}

/**
 * Whether every visible node of the surrounding diagram has been laid out and
 * measured, so its bounds are final. (React Flow's `useNodesInitialized`
 * never settles here: the canvas is controlled without `onNodesChange`, so
 * measurements stay in the internal node lookup this reads.)
 */
export function useDiagramReady(): boolean {
  return useStore((s) => {
    if (s.nodeLookup.size === 0) return false;
    for (const node of s.nodeLookup.values()) {
      if (!node.hidden && !(node.measured.width && node.measured.height)) {
        return false;
      }
    }
    return true;
  });
}

/** The color the canvas is painted with, falling back to the theme token. */
function canvasBackground(domNode: HTMLElement): string | undefined {
  const style = getComputedStyle(domNode);
  const painted = style.backgroundColor;
  if (painted && painted !== "transparent" && painted !== "rgba(0, 0, 0, 0)") {
    return painted;
  }
  const token = style.getPropertyValue("--background").trim();
  return token ? `hsl(${token})` : undefined;
}

/** Save a data URL (or any URL) as a download. */
export function downloadUrl(url: string, filename: string): void {
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  a.click();
}

/** Save text as a download. */
export function downloadText(
  text: string,
  filename: string,
  type = "application/json",
): void {
  const url = URL.createObjectURL(new Blob([text], { type }));
  downloadUrl(url, filename);
  // Let the click's navigation start before revoking.
  setTimeout(() => URL.revokeObjectURL(url), 0);
}
