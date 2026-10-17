// The host's seam into a node's header. A host (a docs site linking components
// to its own concept pages) renders the name and the role line itself, and may
// add trailing content; the
// diagram knows nothing about what they point at. Slots travel by context
// rather than node `data`, so changing them never re-runs the ELK layout.

import { createContext, type ReactNode, useContext } from "react";
import type { GraphNodeDto } from "../types";

export interface NodeSlots {
  /**
   * The node's name line. `fallback` is the text drawn without a slot (the
   * display name, else the id). Return `undefined` to keep it.
   */
  name?: (node: GraphNodeDto, fallback: string) => ReactNode;
  /**
   * The line under the name: the function the node fills (e.g. "Data
   * catalog"). `fallback` is that label. Return `undefined` to keep it.
   */
  role?: (node: GraphNodeDto, fallback: string) => ReactNode;
  /**
   * Trailing header content after the built-in chips, such as a count with
   * its own popover. Nothing is drawn when it returns `undefined`.
   */
  aside?: (node: GraphNodeDto) => ReactNode;
}

export const NodeSlotsContext = createContext<NodeSlots | undefined>(undefined);

export function useNodeSlots(): NodeSlots | undefined {
  return useContext(NodeSlotsContext);
}

/**
 * The element a host's popovers should portal into, or `null` for
 * `document.body`. Inline that is the body; in the expand dialog, a modal that
 * draws above everything in the body, it is a container inside the dialog.
 * Either way it is outside the `.trestle-diagram` style scope, so the host's
 * own styles and tokens apply.
 */
export const DiagramPortalContext = createContext<HTMLElement | null>(null);

export function useDiagramPortal(): HTMLElement | null {
  return useContext(DiagramPortalContext);
}
