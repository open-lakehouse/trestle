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

/** One module in the diagram (mirrors `GraphNodeDto`). Services point at it
 *  through `ServiceNodeDto.module`; it is what selection and the config panel
 *  key on. */
export interface GraphNodeDto {
  id: string;
  display_name?: string | null;
  summary?: string | null;
  category?: string | null;
  /** Role of the module's primary service (e.g. "object_store", "gateway"). */
  role?: string | null;
  /** "in_process" | "host" | "container:<service>". */
  placement?: string | null;
}

/** What a service node stands for (mirrors `ServiceKind`). */
export type ServiceKind = "container" | "external" | "host";

/** A host-published port mapping (mirrors `PortDto`). */
export interface PortDto {
  host: number;
  container: number;
}

/** One runtime service in the diagram (mirrors `ServiceNodeDto`). One-shot
 *  jobs (init/migrate/cert-mint) are never included. */
export interface ServiceNodeDto {
  /** Compose service name, or `"@host"` (Rust `HOST_NODE_ID`) for the
   *  synthetic host/browser node. */
  id: string;
  /** Owning module id; null only for the host node. */
  module?: string | null;
  kind: ServiceKind;
  /** Declared role; null for a fragment-only sidecar (e.g. sts-shim). */
  role?: string | null;
  image?: string | null;
  /** Host-published ports. */
  published: PortDto[];
  /** In-network ports. */
  exposed: number[];
}

/** One gateway route aggregated onto a `route` edge (mirrors `RouteRefDto`). */
export interface RouteRefDto {
  prefix: string;
  host_port: number;
  rewrite?: string | null;
  /** Behind the forward-auth check. */
  gated: boolean;
}

/** A typed, request-direction edge: `from` calls / waits on `to` (mirrors
 *  `TopologyEdgeDto`, serde tag "kind"). */
export type TopologyEdgeDto =
  | { kind: "startup"; from: string; to: string; condition: string }
  | { kind: "route"; from: string; to: string; routes: RouteRefDto[] }
  | { kind: "authz"; from: string; to: string; portal_prefix: string }
  | {
      kind: "emulated";
      from: string;
      to: string;
      hosts: string[];
      port: number;
    }
  | { kind: "ingress"; from: string; to: string; host_ports: number[] };

/** The edge kinds, for legends and filters. */
export type TopologyEdgeKind = TopologyEdgeDto["kind"];

/** The runtime topology (mirrors `GraphDto`). */
export interface GraphDto {
  nodes: GraphNodeDto[];
  services: ServiceNodeDto[];
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
