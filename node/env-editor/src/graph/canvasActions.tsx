// The canvas's editing seam. A host that passes `actions` to
// `MarkitectureCanvas` turns its nodes interactive (a hover action bar, an
// "auto" chip); without it the canvas stays a read-only diagram. The actions
// travel by context rather than node `data`, so changing them never re-runs
// the ELK layout.

import { createContext, useContext } from "react";
import type { Origin } from "../state/origin";
import type { ModuleDto } from "../types";

export interface CanvasActions {
  /** Why a component is in the environment; only explicit picks can be removed. */
  originOf: (id: string) => Origin;
  /** Whether the component exposes any settings (knobs). */
  hasSettings: (id: string) => boolean;
  /** Catalog modules that could take this component's place (same role). */
  alternativesOf: (id: string) => ModuleDto[];
  /** Open the component's settings. */
  configure: (id: string) => void;
  remove: (id: string) => void;
  swap: (fromId: string, toId: string) => void;
  /** Open the provider's resources to add a database / bucket. */
  addResource: (providerId: string) => void;
  /** Drop a resource the user added (one no component consumes). */
  removeResource: (kind: string, name: string) => void;
}

export interface CanvasContextValue {
  actions: CanvasActions;
  /** The node under the pointer (or its action bar). */
  hoveredId: string | undefined;
  /** Mark a node hovered. */
  hover: (id: string) => void;
  /** Clear the hover after a short grace period, so the pointer can travel
   *  from a node onto its action bar. */
  leave: () => void;
}

export const CanvasContext = createContext<CanvasContextValue | null>(null);

/** The canvas's editing context, or `null` when the canvas is read-only. */
export function useCanvasContext(): CanvasContextValue | null {
  return useContext(CanvasContext);
}
