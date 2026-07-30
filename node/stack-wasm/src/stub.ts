// Build-time stand-in for `@open-lakehouse/stack-wasm` in default builds.
//
// The real entry (./index.ts) imports the gitignored wasm-bindgen artifact under
// node/stack-wasm/pkg/, which only exists after `just build-stack-wasm`. Default
// builds must not resolve it, so node/env-app/vite.config.ts aliases the package
// here unless VITE_ENABLE_WASM=true. This stub serves the captured fixture so the
// app (and Storybook) render the full editor with no wasm — the catalog is real,
// and `plan()` returns a representative pre-planned environment regardless of the
// exact selection.

import { FIXTURE_CATALOG, FIXTURE_PLAN } from "./fixtures";
import type { CatalogDto, Planner, PlanResult, Selection } from "./types";

export type {
  ArtifactsDto,
  CatalogDto,
  ClusterDto,
  EdgeDto,
  Endpoint,
  GatewayDto,
  GraphDto,
  GraphNodeDto,
  Knob,
  KnobKind,
  ListenerDto,
  ModuleDto,
  Planner,
  PlanResult,
  RouteDto,
  Selection,
  ServiceSpec,
} from "./types";

/**
 * A fixture-backed {@link Planner}. `catalog()` returns the real captured
 * catalog; `plan()` returns the captured plan for any non-empty selection and
 * rejects on an empty one (so the editor's error path is still exercisable).
 * Swap this for `createWasmPlanner()` (VITE_ENABLE_WASM=true) for live planning.
 */
export function createWasmPlanner(): Planner {
  return {
    async catalog(): Promise<CatalogDto> {
      return FIXTURE_CATALOG;
    },
    async plan(selection: Selection): Promise<PlanResult> {
      if (
        selection.modules.length === 0 &&
        selection.capabilities.length === 0
      ) {
        throw new Error(
          "select at least one technology or capability to generate an environment",
        );
      }
      return FIXTURE_PLAN;
    },
  };
}
