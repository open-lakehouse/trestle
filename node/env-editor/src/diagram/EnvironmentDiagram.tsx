import { cn } from "@open-lakehouse/ui-kit";
import { type ColorMode, Panel, useReactFlow, useStore } from "@xyflow/react";
import { Maximize2, X } from "lucide-react";
import { type ReactNode, useEffect, useMemo, useRef, useState } from "react";
import {
  ConnectionLegend,
  MarkitectureCanvas,
} from "../graph/MarkitectureCanvas";
import type { GraphDto } from "../types";
import { useDiagramReady } from "./exportImage";
import { parseGraphSnapshot } from "./snapshot";

export interface EnvironmentDiagramProps {
  /**
   * A graph snapshot (the parsed `*.trestle-graph.json`). It is validated
   * here, so raw imported JSON can be passed straight through.
   */
  snapshot?: unknown;
  /** A graph to draw directly, instead of a snapshot. */
  graph?: GraphDto;
  /**
   * Light/dark rendering. Defaults to `"system"`; a host with its own theme
   * toggle passes its active theme.
   */
  colorMode?: ColorMode;
  /**
   * A fixed height in pixels. By default the embed takes the diagram's own
   * aspect ratio at the available width, clamped to `minHeight`–`maxHeight`.
   */
  height?: number;
  /** Lower bound for the fitted height. Defaults to 240. */
  minHeight?: number;
  /** Upper bound for the fitted height. Defaults to 640. */
  maxHeight?: number;
  /**
   * Whether to show the connection-kind legend, as a row under the diagram
   * (inline an overlay would cover the fitted nodes). Defaults to `true`.
   */
  legend?: boolean;
  /**
   * Whether an expand button opens the diagram in a full-window, pannable and
   * zoomable dialog. Defaults to `true`.
   */
  expandable?: boolean;
  /**
   * The diagram's accessible name. Defaults to the snapshot's `title`.
   */
  label?: string;
  className?: string;
}

/**
 * A read-only environment diagram for embedding in a page (a blog post, docs):
 * the editor's own canvas, drawn from a frozen graph snapshot. Inline it is a
 * fixed, fitted picture that never captures the page's scroll; the expand
 * button opens a dialog to pan and zoom.
 *
 * Styles ship separately (`@open-lakehouse/trestle-diagram/styles.css`) and are
 * scoped to the `.trestle-diagram` root this renders.
 */
export function EnvironmentDiagram({
  snapshot,
  graph,
  colorMode = "system",
  height,
  minHeight = 240,
  maxHeight = 640,
  legend = true,
  expandable = true,
  label,
  className,
}: EnvironmentDiagramProps) {
  const resolved = useMemo(() => {
    if (graph) return { graph, title: undefined };
    try {
      const parsed = parseGraphSnapshot(snapshot);
      return { graph: parsed.graph, title: parsed.title };
    } catch (e) {
      return { error: e instanceof Error ? e.message : String(e) };
    }
  }, [snapshot, graph]);
  const [fitted, setFitted] = useState<number>();
  const [expanded, setExpanded] = useState(false);
  const dark = useDark(colorMode);

  // The whole root takes the theme (not just the canvas, which React Flow
  // themes itself), so the legend row and expand dialog match it.
  const root = cn("trestle-diagram", dark && "dark", className);

  if ("error" in resolved) {
    return (
      <div className={root}>
        <div
          role="alert"
          className="rounded-lg border border-destructive/40 bg-card p-4 text-sm text-destructive"
        >
          Could not render the environment diagram: {resolved.error}
        </div>
      </div>
    );
  }

  return (
    <figure
      className={root}
      aria-label={label ?? resolved.title ?? "Environment diagram"}
    >
      <div
        className="relative w-full overflow-hidden rounded-lg"
        style={{ height: height ?? fitted ?? minHeight }}
      >
        <MarkitectureCanvas
          graph={resolved.graph}
          colorMode={colorMode}
          interactive={false}
          legend={false}
        >
          <FitViewport />
          {height === undefined && (
            <FitHeight min={minHeight} max={maxHeight} onHeight={setFitted} />
          )}
          {expandable && (
            <Panel position="bottom-right">
              <ChromeButton
                label="Expand diagram"
                onClick={() => setExpanded(true)}
              >
                <Maximize2 className="h-4 w-4" />
              </ChromeButton>
            </Panel>
          )}
        </MarkitectureCanvas>
      </div>
      {legend && <ConnectionLegend graph={resolved.graph} layout="row" />}
      {expandable && (
        <ExpandedDialog open={expanded} onClose={() => setExpanded(false)}>
          <MarkitectureCanvas
            graph={resolved.graph}
            colorMode={colorMode}
            legend={legend}
          >
            <Panel position="top-left">
              <ChromeButton label="Close" onClick={() => setExpanded(false)}>
                <X className="h-4 w-4" />
              </ChromeButton>
            </Panel>
          </MarkitectureCanvas>
        </ExpandedDialog>
      )}
    </figure>
  );
}

/** Whether `colorMode` resolves to dark, following the OS for `"system"`. */
function useDark(colorMode: ColorMode): boolean {
  const query = "(prefers-color-scheme: dark)";
  const [systemDark, setSystemDark] = useState(
    () => typeof window !== "undefined" && window.matchMedia(query).matches,
  );
  useEffect(() => {
    if (colorMode !== "system") return;
    const media = window.matchMedia(query);
    const update = () => setSystemDark(media.matches);
    update();
    media.addEventListener("change", update);
    return () => media.removeEventListener("change", update);
  }, [colorMode]);
  return colorMode === "dark" || (colorMode === "system" && systemDark);
}

/** The diagram's bounds-derived height at the pane's width, once measured. */
function FitHeight({
  min,
  max,
  onHeight,
}: {
  min: number;
  max: number;
  onHeight: (height: number) => void;
}) {
  const { getNodes, getNodesBounds } = useReactFlow();
  const initialized = useDiagramReady();
  const width = useStore((s) => s.width);

  useEffect(() => {
    if (!initialized || width === 0) return;
    const nodes = getNodes();
    if (nodes.length === 0) return;
    const bounds = getNodesBounds(nodes);
    if (bounds.width === 0) return;
    // A little extra over the pure aspect ratio leaves room for the fit padding.
    const ideal = (width * bounds.height) / bounds.width + 32;
    onHeight(Math.round(Math.min(max, Math.max(min, ideal))));
  }, [initialized, width, getNodes, getNodesBounds, min, max, onHeight]);

  return null;
}

/**
 * Refit whenever the pane resizes (the fitted height settling, a responsive
 * page): an embed has no controls to recover a stale viewport.
 */
function FitViewport() {
  const { fitView } = useReactFlow();
  const initialized = useDiagramReady();
  const width = useStore((s) => s.width);
  const height = useStore((s) => s.height);

  useEffect(() => {
    if (!initialized || width === 0 || height === 0) return;
    const raf = requestAnimationFrame(() => {
      void fitView({ padding: 0.08 });
    });
    return () => cancelAnimationFrame(raf);
  }, [initialized, width, height, fitView]);

  return null;
}

/**
 * A native modal `<dialog>`. Unlike a portal-based dialog it stays inside the
 * `.trestle-diagram` root in the DOM, so the scoped styles still apply.
 */
function ExpandedDialog({
  open,
  onClose,
  children,
}: {
  open: boolean;
  onClose: () => void;
  children: ReactNode;
}) {
  const ref = useRef<HTMLDialogElement>(null);

  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    if (open && !dialog.open) dialog.showModal();
    if (!open && dialog.open) dialog.close();
  }, [open]);

  return (
    // biome-ignore lint/a11y/useKeyWithClickEvents: the click only catches the backdrop; Escape already closes a modal <dialog> natively.
    <dialog
      ref={ref}
      aria-label="Environment diagram"
      className="m-auto h-[90vh] max-h-none w-[92vw] max-w-none overflow-hidden rounded-xl border border-border bg-background p-0 shadow-2xl backdrop:bg-black/60"
      // Escape fires `close`; keep React's state in step with it.
      onClose={onClose}
      // A click on the backdrop lands on the dialog element itself.
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      {open && <div className="relative h-full w-full">{children}</div>}
    </dialog>
  );
}

function ChromeButton({
  label,
  onClick,
  children,
}: {
  label: string;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      onClick={onClick}
      className="inline-flex h-8 w-8 items-center justify-center rounded-md border border-border bg-card/95 text-muted-foreground shadow-sm transition-colors hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
    >
      {children}
    </button>
  );
}
