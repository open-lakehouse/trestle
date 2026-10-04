// The editor's public types. These are re-exported from
// `@open-lakehouse/stack-wasm`'s type surface (the DTO mirror of the Rust
// planner), imported with `import type` only — so this component has NO runtime
// dependency on the wasm package and stays headless: a host injects any
// `Planner` (real wasm or fixtures) through the seam.

export type {
  AuthDto,
  CatalogDto,
  ClusterDto,
  Endpoint,
  ExposedDto,
  ExposedKind,
  GatewayDto,
  GraphDto,
  GraphNodeDto,
  Knob,
  KnobKind,
  ListenerDto,
  ModuleDto,
  NodeKind,
  OutputFile,
  Planner,
  PlanResult,
  ProvisionedDto,
  RouteDto,
  Selection,
  ServiceSpec,
  TlsDto,
  TopologyEdgeDto,
  TopologyEdgeKind,
  VirtualHostDto,
} from "@open-lakehouse/stack-wasm/types";
