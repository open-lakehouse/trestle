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
import type {
  CatalogDto,
  PlanContext,
  Planner,
  PlanResult,
  Selection,
} from "./types";

export type {
  AuthDto,
  Bundle,
  BundleFile,
  CatalogDto,
  ClusterDto,
  Endpoint,
  ExposedDto,
  ExposedKind,
  GatewayDto,
  GraphDto,
  GraphNodeDto,
  HostedConnection,
  HostedKind,
  HostedModuleDto,
  HostedResource,
  HostPortDto,
  Knob,
  KnobKind,
  ListenerDto,
  LocalCredentialDto,
  ModuleDto,
  NodeKind,
  OutputFile,
  PlanContext,
  Planner,
  PlanResult,
  PolicyDto,
  ProvisionedDto,
  RouteDto,
  Selection,
  ServiceSpec,
  SurfaceDto,
  SurfaceKind,
  TargetDto,
  TlsDto,
  TopologyEdgeDto,
  TopologyEdgeKind,
  VirtualHostDto,
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
    async plan(
      selection: Selection,
      _target?: string,
      context?: PlanContext,
    ): Promise<PlanResult> {
      if (
        selection.modules.length === 0 &&
        selection.capabilities.length === 0
      ) {
        throw new Error(
          "select at least one technology or capability to generate an environment",
        );
      }
      // The fixture's files are fixed; its host ports follow the context, so a
      // workspace's members still look like they sit on their own slots.
      const gateway = context?.gateway_host_port ?? 9080;
      const admin = context?.gateway_admin_port ?? 9901;
      return {
        ...FIXTURE_PLAN,
        host_ports: [
          { port: gateway, owner: "envoy" },
          { port: admin, owner: "envoy" },
        ],
      };
    },
  };
}
