// The planner seam: how a host injects the environment engine into the editor.
//
// The editor never imports a wasm runtime; it calls a `Planner` provided through
// React context. The demo app injects `createWasmPlanner()` (real in-browser
// planning); Storybook and tests inject a fixture planner. This mirrors the
// transport seam pattern the sibling headwaters lineage UI uses.

import { createContext, useContext } from "react";
import type { Planner } from "./types";

const PlannerContext = createContext<Planner | null>(null);

export interface PlannerProviderProps {
  planner: Planner;
  children: React.ReactNode;
}

/** Provide the `Planner` the editor's `catalog()` / `plan()` calls resolve to. */
export function PlannerProvider({ planner, children }: PlannerProviderProps) {
  return (
    <PlannerContext.Provider value={planner}>
      {children}
    </PlannerContext.Provider>
  );
}

/**
 * The injected `Planner`. Throws if used outside a `PlannerProvider` (or the
 * `planner` prop on `EnvironmentEditor`), which is a wiring bug, not a runtime
 * condition.
 */
export function usePlanner(): Planner {
  const planner = useContext(PlannerContext);
  if (!planner) {
    throw new Error(
      "usePlanner must be used within a <PlannerProvider> (or pass `planner` to <EnvironmentEditor>)",
    );
  }
  return planner;
}

export { PlannerContext };
