// TypeScript mirror of the Rust DTOs in
// `crates/stack-topology-wasm/src/dto.rs`. Hand-authored (serde-wasm-bindgen
// produces untyped JS objects), and the single source of truth for both the
// wasm client and the `@open-lakehouse/env-editor` component. Keep these shapes
// in sync with `dto.rs` — its native unit tests pin the projection.

/** The kind of value a knob holds (mirrors `KnobKind`, serde tag "kind",
 *  snake_case). Drives which form control the editor renders. */
export type KnobKind =
  | { kind: "string" }
  | { kind: "bool" }
  | { kind: "enum"; options: string[] }
  | { kind: "integer"; min?: number | null; max?: number | null }
  | { kind: "port" };

/** One user-tunable configuration value a module exposes (mirrors `Knob`). */
export interface Knob {
  key: string;
  title?: string | null;
  kind: KnobKind;
  default?: string | null;
  required: boolean;
  help?: string | null;
  aliases?: string[];
}

/** One selectable module, projected for the picker (mirrors `ModuleDto`). */
export interface ModuleDto {
  id: string;
  display_name?: string | null;
  summary?: string | null;
  category?: string | null;
  /** The capability this module provides (e.g. "experiment_tracking"). */
  provider_of?: string | null;
  requires: string[];
  conflicts_with: string[];
  knobs: Knob[];
}

/** The user's environment choices (mirrors the Rust `Selection`). This is the
 *  JSON the editor produces and hands to `plan()`. */
export interface Selection {
  modules: string[];
  capabilities: string[];
  /** module id -> (knob key -> value); values are always strings. */
  knob_overrides: Record<string, Record<string, string>>;
  extra_resources?: unknown[];
}

/** The selectable catalog (mirrors `CatalogDto`). */
export interface CatalogDto {
  modules: ModuleDto[];
  default_selection: Selection;
}

/** What a diagram node stands for (mirrors `NodeKind`). */
export type NodeKind = "clients" | "gateway" | "component" | "external";

/** What kind of surface the gateway exposes (mirrors `ExposedKind`). */
export type ExposedKind = "api" | "ui" | "service";

/** One endpoint the gateway exposes to clients (mirrors `ExposedDto`). */
export interface ExposedDto {
  /** The backend component serving it. */
  module: string;
  endpoint: string;
  kind: ExposedKind;
  /** Client-facing prefix (`/` for a whole-service listener). */
  prefix: string;
  host_port: number;
  /** Behind forward-auth. */
  gated: boolean;
}

/** A resource a provider provisions for a consumer (mirrors `ProvisionedDto`). */
export interface ProvisionedDto {
  resource: string;
  name: string;
}

/** One functional component (a module), or the clients node (mirrors
 *  `GraphNodeDto`). Helper containers (STS shim, init jobs, …) are part of
 *  their component and never appear on their own. */
export interface GraphNodeDto {
  /** Module id, or `"@clients"` (Rust `CLIENTS_NODE_ID`). */
  id: string;
  kind: NodeKind;
  /** The implementation's name (e.g. "RustFS (local S3 + STS)"). */
  display_name?: string | null;
  summary?: string | null;
  category?: string | null;
  /** The function it fills (e.g. "object_store", "auth"). */
  role?: string | null;
  /** "in_process" | "host" | "container:<service>". */
  placement?: string | null;
  /** For a provider: the APIs it offers (e.g. "S3 + STS"). */
  offers: string[];
  /** For the gateway: everything it exposes, grouped by backend. */
  exposes: ExposedDto[];
  /** For a provider: the databases / buckets it provisions. */
  provisions: ProvisionedDto[];
}

/** A typed, request-direction edge: `from` calls `to` (mirrors
 *  `TopologyEdgeDto`, serde tag "kind"). */
export type TopologyEdgeDto =
  | { kind: "access"; from: string; to: string; host_ports: number[] }
  | { kind: "route"; from: string; to: string; gated: boolean }
  | { kind: "authenticates"; from: string; to: string; portal_prefix: string }
  | {
      kind: "uses";
      from: string;
      to: string;
      role: string;
      protocol: string;
      resources: string[];
    };

/** The edge kinds, for legends and filters. */
export type TopologyEdgeKind = TopologyEdgeDto["kind"];

/** The functional topology (mirrors `GraphDto`). */
export interface GraphDto {
  nodes: GraphNodeDto[];
  edges: TopologyEdgeDto[];
}

/** A gateway route (mirrors `RouteDto`). */
export interface RouteDto {
  prefix: string;
  cluster: string;
  rewrite?: string | null;
}

/** A gateway listener and its routes (mirrors `ListenerDto`). */
export interface ListenerDto {
  host_port: number;
  routes: RouteDto[];
}

/** An upstream cluster (mirrors `ClusterDto`). */
export interface ClusterDto {
  name: string;
  host: string;
  port: number;
}

/** Forward-auth wiring (mirrors `AuthDto`). */
export interface AuthDto {
  cluster: string;
  portal_prefix: string;
}

/** The hostnames one cluster answers on the TLS listener (mirrors
 *  `VirtualHostDto`). */
export interface VirtualHostDto {
  cluster: string;
  hosts: string[];
}

/** The emulated-hostname TLS listener (mirrors `TlsDto`). */
export interface TlsDto {
  port: number;
  hosts: string[];
  virtual_hosts: VirtualHostDto[];
}

/** The gateway layout (mirrors `GatewayDto`). */
export interface GatewayDto {
  listeners: ListenerDto[];
  clusters: ClusterDto[];
  admin_port: number;
  auth?: AuthDto | null;
  tls?: TlsDto | null;
}

/** One materialized file from a plan (mirrors `OutputFileDto`). */
export interface OutputFile {
  path: string;
  contents: string;
}

/** One endpoint a service offers (mirrors `Endpoint`; loosely typed — the
 *  editor only surfaces the shape for detail views). */
export interface Endpoint {
  id: string;
  scheme: string;
  internal_port: number;
  [key: string]: unknown;
}

/** One service a module contributes (mirrors `ServiceSpec`). */
export interface ServiceSpec {
  name: string;
  role: string;
  placement: unknown;
  endpoints: Endpoint[];
  depends_on: string[];
  base_path?: string;
}

/** The full plan result (mirrors `PlanResultDto`). */
export interface PlanResult {
  graph: GraphDto;
  services: Record<string, ServiceSpec[]>;
  gateway: GatewayDto;
  files: OutputFile[];
}

/**
 * The headless planner seam. The `@open-lakehouse/env-editor` component depends
 * only on this interface (and the DTO types above), never on the wasm runtime,
 * so it stays reusable and testable with fixtures. The app injects a real
 * implementation via `createWasmPlanner()`; Storybook/tests inject a fixture one.
 */
export interface Planner {
  /** The selectable catalog for the picker. */
  catalog(): Promise<CatalogDto>;
  /** Plan a selection into a full result, or reject with a `PlanError` message. */
  plan(selection: Selection): Promise<PlanResult>;
}
