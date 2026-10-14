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
  /** A tuning knob with a safe default; UIs hide it unless the user opts in. */
  advanced?: boolean;
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
  /** The resource kinds it provisions on request (e.g. "relational_db"). */
  resource_kinds: string[];
  /** What it calls each kind it provisions (e.g. Azurite's "containers"). */
  resource_nouns: ResourceNounDto[];
  knobs: Knob[];
}

/** The user's environment choices (mirrors the Rust `Selection`). This is the
 *  JSON the editor produces and hands to `plan()`. */
export interface Selection {
  modules: string[];
  capabilities: string[];
  /** module id -> (knob key -> value); values are always strings. */
  knob_overrides: Record<string, Record<string, string>>;
  /** Environment-level resources no module consumes (extra databases, buckets). */
  extra_resources?: ExtraResource[];
  /** Providers the environment uses but does not run (a managed cloud database
   *  or object store), by module id. */
  external?: Record<string, ExternalResource>;
  /** Whether to generate the Python client workspace (`client/`), and with
   *  which example notebooks (mirrors `ClientSelection`). Off when omitted. */
  client?: ClientSelection;
  /** How the always-on gateway guards access (mirrors `GatewaySettings`).
   *  Omitted means open. */
  gateway?: GatewaySettings;
}

/** Python client workspace options (mirrors the Rust `ClientSelection`). */
export interface ClientSelection {
  /** Generate `client/pyproject.toml`, a README and a connection notebook. */
  python?: boolean;
  /** Also emit the selected modules' example notebooks; implies `python`. */
  examples?: boolean;
}

/** Gateway access (mirrors the Rust `GatewaySettings`). Each setting pulls in
 *  its provider by role; `policy` implies `auth`. */
export interface GatewaySettings {
  /** Require sign-in (the `auth`-role provider, Authelia). */
  auth: boolean;
  /** Authorize each signed-in request (the `policy`-role provider, OPA). */
  policy: boolean;
}

/** A provider marked external (mirrors the Rust `ExternalResource`). */
export interface ExternalResource {
  /** The cloud environment it lives in (e.g. "aws"); groups nodes on the canvas. */
  cloud?: string | null;
  connection: ExternalConnection;
}

/** How consumers reach an external resource (mirrors `ExternalConnection`).
 *  Strings may hold `{name}` and `${VAR}` secret references. */
export type ExternalConnection =
  | {
      kind: "relational_db";
      host: string;
      port?: number;
      database?: string;
      username: string;
      password: string;
    }
  | {
      kind: "object_store";
      endpoint: string;
      uri?: string | null;
      bucket?: string;
      credential:
        | {
            flavour: "s3";
            access_key_id: string;
            secret_access_key: string;
            region: string;
            role_arn?: string | null;
          }
        | { flavour: "azure_blob"; connection_string: string };
    };

/** An extra resource to provision (mirrors the Rust `ExtraResource`). */
export interface ExtraResource {
  /** The resource kind (e.g. "relational_db", "object_store"). */
  resource: string;
  /** The database / bucket name. */
  name: string;
  /** Pin the provider module; otherwise the planner chooses. */
  provider?: string | null;
}

/** A deploy target an environment can run on (mirrors `TargetDto`). */
export interface TargetDto {
  /** Stable id (e.g. "compose"), what `Planner.plan` takes. */
  id: string;
  display_name: string;
}

/** The selectable catalog (mirrors `CatalogDto`). */
export interface CatalogDto {
  modules: ModuleDto[];
  default_selection: Selection;
  /** The deploy targets, the default (Docker Compose) first. */
  targets: TargetDto[];
}

/** What a diagram node stands for (mirrors `NodeKind`). */
export type NodeKind = "clients" | "gateway" | "component" | "external";

/** What kind of endpoint the gateway exposes (mirrors `ExposedKind`). */
export type ExposedKind = "api" | "ui" | "service";

/** What a gateway surface is for (mirrors `SurfaceKind`): the shared
 *  platform API/UI surface, or a dedicated service endpoint (e.g. S3) whose
 *  clients need their own origin. */
export type SurfaceKind = "platform" | "service";

/** One endpoint the gateway exposes to clients (mirrors `ExposedDto`). */
export interface ExposedDto {
  /** The backend component serving it. */
  module: string;
  endpoint: string;
  kind: ExposedKind;
  /** Client-facing prefix (`/` for a whole-service listener). */
  prefix: string;
  /** Behind forward-auth. */
  gated: boolean;
}

/** One gateway listener — locally a host port, hosted its own subdomain
 *  (mirrors `SurfaceDto`). */
export interface SurfaceDto {
  host_port: number;
  kind: SurfaceKind;
  /** For a service surface: the protocols it speaks (e.g. "S3 + STS"). */
  protocols: string[];
  /** The endpoints it exposes, grouped by backend. */
  exposes: ExposedDto[];
}

/** What a provider calls one kind of resource it provisions (mirrors
 *  `ResourceNounDto`). */
export interface ResourceNounDto {
  kind: string;
  singular: string;
  plural: string;
}

/** A resource a provider provisions (mirrors `ProvisionedDto`). */
export interface ProvisionedDto {
  resource: string;
  name: string;
  /** The components that use it; empty for an extra resource. */
  consumers: string[];
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
  /** "in_process" | "host" | "container:<service>" | "external:<origin>". */
  placement?: string | null;
  /** For a resource provided by an external cloud: its environment's label
   *  (e.g. "aws"; "" when unnamed). External nodes are grouped by it. */
  cloud?: string | null;
  /** For a provider: the APIs it offers (e.g. "S3 + STS"). */
  offers: string[];
  /** For the gateway: one surface per listener, platform first. */
  surfaces: SurfaceDto[];
  /** For a provider: the databases / buckets it provisions. */
  provisions: ProvisionedDto[];
  /** For a provider: what it calls each kind of resource it provisions. */
  resource_nouns: ResourceNounDto[];
}

/** A typed, request-direction edge: `from` calls `to` (mirrors
 *  `TopologyEdgeDto`, serde tag "kind"). */
export type TopologyEdgeDto =
  | { kind: "access"; from: string; to: string; host_port: number }
  | {
      kind: "route";
      from: string;
      to: string;
      host_port: number;
      gated: boolean;
    }
  | {
      kind: "authenticates";
      from: string;
      to: string;
      host_port: number;
      portal_prefix: string;
    }
  | { kind: "authorizes"; from: string; to: string; host_port: number }
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

/** Authorization-policy wiring (mirrors `PolicyDto`). */
export interface PolicyDto {
  cluster: string;
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
  policy?: PolicyDto | null;
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
  /** Plan a selection for a deploy target (default: Docker Compose) into a full
   *  result, or reject with a `PlanError` message. */
  plan(selection: Selection, target?: string): Promise<PlanResult>;
}
