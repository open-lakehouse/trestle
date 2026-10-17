// The host's seam into a node's header. A host (a docs site linking components
// to its own concept pages) renders the name and the role line itself; the
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
}

export const NodeSlotsContext = createContext<NodeSlots | undefined>(undefined);

export function useNodeSlots(): NodeSlots | undefined {
  return useContext(NodeSlotsContext);
}

/**
 * The element a host's popovers should portal into: the diagram's root inline,
 * the expand dialog while it is open. A popover portaled to `document.body`
 * is clipped by the inline frame or drawn under the modal dialog's top layer.
 * Both containers sit inside the `.trestle-diagram` root, so a host's popover
 * carries its own styles. `null` before the container mounts.
 */
export const DiagramPortalContext = createContext<HTMLElement | null>(null);

export function useDiagramPortal(): HTMLElement | null {
  return useContext(DiagramPortalContext);
}
