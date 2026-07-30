/// <reference path="./pkg.d.ts" />
// Public surface of `@open-lakehouse/stack-wasm`: a `Planner` backed by the
// in-browser `stack-topology-wasm` module. The whole catalog -> plan -> render
// pipeline is pure Rust compiled to wasm, and each call is fast and stateless,
// so — unlike a heavy query engine — there is no Web Worker: we init the module
// once on the main thread and call into it directly.
//
// The wasm artifact is imported via the BARE specifier "stack-topology-wasm-pkg"
// (typed by ./pkg.d.ts, produced by `just build-stack-wasm`, gitignored). Default
// builds alias THIS PACKAGE to ./stub.ts (see node/env-app/vite.config.ts), so
// the artifact is only resolved when VITE_ENABLE_WASM=true.

import initWasm, { catalog, init, plan } from "stack-topology-wasm-pkg";
import type { CatalogDto, Planner, PlanResult, Selection } from "./types";

export type {
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
  OutputFile,
  Planner,
  PlanResult,
  RouteDto,
  Selection,
  ServiceSpec,
} from "./types";

let ready: Promise<void> | undefined;

/** Initialize the wasm module exactly once (idempotent across planner calls). */
function ensureReady(): Promise<void> {
  ready ??= initWasm().then(() => {
    init();
  });
  return ready;
}

/**
 * Build a {@link Planner} backed by the in-browser wasm engine. `catalog()` and
 * `plan()` both ensure the module is initialized first; `plan()` serializes the
 * selection to JSON for the boundary and surfaces a Rust `PlanError` as a
 * rejected promise carrying the message.
 */
export function createWasmPlanner(): Planner {
  return {
    async catalog(): Promise<CatalogDto> {
      await ensureReady();
      return catalog() as CatalogDto;
    },
    async plan(selection: Selection): Promise<PlanResult> {
      await ensureReady();
      // The wasm `plan` throws (rejects) with a string message on a plan error;
      // let it propagate so the UI can render it inline.
      return plan(JSON.stringify(selection)) as PlanResult;
    },
  };
}
