// TypeScript mirror of the Rust DTOs in
// `crates/stack-topology/src/projection/mod.rs`. Hand-authored (serde-wasm-bindgen
// produces untyped JS objects), and the single source of truth for both the
// wasm client and the `@open-lakehouse/env-editor` component. Keep these shapes
// in sync with `projection/mod.rs` — its native unit tests pin the projection.

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
  /** The implementation the module packages, as an architecture-model element
   *  id (e.g. "postgresql"). Identity only. */
  model?: string | null;
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
  /** The init jobs and sidecars the module may run beside its services. */
  helpers: HelperService[];
  /** Set for a hosted provider: a managed database or cloud object store the
   *  stack uses but does not run. */
  hosted?: HostedModuleDto | null;
}

/** A container a module runs to support its services (mirrors
 *  `HelperService`). */
export interface HelperService {
  /** The compose service name. */
  name: string;
  kind: "init" | "sidecar";
}

/** What a hosted provider stands for (mirrors `HostedKind`). */
export type HostedKind = "postgres" | "s3" | "azure_blob";

/** A hosted provider's kind and default configuration (mirrors
 *  `HostedModuleDto`). */
export interface HostedModuleDto {
  kind: HostedKind;
  /** The configuration used while the selection has no `hosted` entry for it. */
  default: HostedResource;
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
  /** How each hosted provider is reached, by module id (e.g. "aws-s3"). A
   *  hosted module without an entry uses its default. */
  hosted?: Record<string, HostedResource>;
  /** Whether to generate the Python client workspace (`client/`), and with
   *  which example notebooks (mirrors `ClientSelection`). Off when omitted. */
  client?: ClientSelection;
  /** Whether the gateway runs and how it guards access (mirrors
   *  `GatewaySettings`). Omitted means a gateway that is open. */
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
  /** Run the gateway; omitted means on. Off publishes each service on its own
   *  host port, and rules out `auth` and `policy`. */
  enabled?: boolean;
  /** Require sign-in (the `auth`-role provider, Authelia). */
  auth: boolean;
  /** Authorize each signed-in request (the `policy`-role provider, OPA). */
  policy: boolean;
}

/** Who runs a hosted resource (mirrors `HostedVendor`): it decides the
 *  resource's setup steps. `generic` fits every kind. */
export type HostedVendor =
  | "generic"
  | "neon"
  | "aws-rds"
  | "azure-postgres"
  | "aws-s3"
  | "azure-storage";

/** How a hosted provider is configured (mirrors the Rust `HostedResource`). */
export interface HostedResource {
  /** The cloud environment it lives in (e.g. "aws"); groups nodes on the canvas.
   *  Defaults from the vendor. */
  cloud?: string | null;
  /** Who runs it; defaults to the kind's usual vendor. */
  vendor?: HostedVendor | null;
  connection: HostedConnection;
}

/** How consumers reach a hosted resource, one variant per `HostedKind`
 *  (mirrors `HostedConnection`). Strings may hold `{name}` and `${VAR}`
 *  secret references. */
export type HostedConnection =
  | {
      kind: "postgres";
      host: string;
      port?: number;
      database?: string;
      username: string;
      password: string;
      /** libpq `sslmode`; defaults to `require` for managed vendors. */
      ssl_mode?: string | null;
    }
  | {
      kind: "s3";
      region?: string;
      /** Defaults to `https://s3.<region>.amazonaws.com`. */
      endpoint?: string | null;
      bucket?: string;
      access_key_id: string;
      secret_access_key: string;
      role_arn?: string | null;
    }
  | {
      kind: "azure_blob";
      account: string;
      container?: string;
      connection_string: string;
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
  /** The in-stack credentials a bundle can be given fresh values for. Absent
   *  from catalogs captured before bundles existed. */
  local_credentials?: LocalCredentialDto[];
}

/** One in-stack credential (mirrors `LocalCredentialDto`): a value for it is
 *  `min_len..=max_len` characters of `[A-Za-z0-9_-]`. */
export interface LocalCredentialDto {
  key: string;
  description: string;
  min_len: number;
  max_len: number;
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
  /** The implementation the component is, as an architecture-model element id;
   *  null for the clients node. Absent in snapshots saved before it existed. */
  model?: string | null;
  kind: NodeKind;
  /** The implementation's name (e.g. "RustFS"). */
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
 *  `TopologyEdgeDto`, serde tag "kind"). `key` (`kind:from→to:qualifier`) is
 *  unique within its graph and stable across re-plans. */
export type TopologyEdgeDto = { key: string } & (
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
    }
);

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

/** One file of a {@link Bundle} (mirrors `BundleFileDto`). */
export interface BundleFile {
  path: string;
  contents: string;
  /** It holds credentials: write it owner-only. */
  sensitive: boolean;
  /** It is a script to run: write it executable. */
  executable: boolean;
}

/** Every file of a rendered environment, credential-bearing ones included
 *  (mirrors `BundleDto`), for packaging as a download. */
export interface Bundle {
  env_name: string;
  files: BundleFile[];
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

/** When a deployment step happens, relative to starting the stack (mirrors
 *  `DeployPhase`). */
export type DeployPhase = "provision" | "configure" | "start" | "verify";

/** One deployment step, in deployment order (mirrors `DeployStepDto`). */
export interface DeployStepDto {
  /** 1-based position in the deployment order. */
  order: number;
  phase: DeployPhase;
  /** The contributing module; absent for the deploy target's own step. */
  module?: string;
  /** Unique within its module. */
  id: string;
  title: string;
  /** What to do, as Markdown. */
  instructions: string;
  /** The path of the script that does it (among the plan's files). */
  script?: string;
  /** The script's own commands, without its preamble: what a reader pastes into a
   *  shell at the environment's root once that shell has loaded `secrets.env`. */
  commands?: string;
}

/** The full plan result (mirrors `PlanResultDto`). */
export interface PlanResult {
  graph: GraphDto;
  /** `graph` as a Mermaid flowchart, for Markdown (without a code fence). */
  mermaid: string;
  services: Record<string, ServiceSpec[]>;
  gateway: GatewayDto;
  files: OutputFile[];
  /** The deployment steps, in order (collected into the root README.md). */
  deploy: DeployStepDto[];
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
  /** Render every file of a selection's environment, with the in-stack
   *  credentials set to `credentials` (keyed by the catalog's
   *  `local_credentials`; missing ones keep their development defaults), or
   *  reject with a message. Optional: a planner without it (e.g. a read-only
   *  fixture) can't produce a download. */
  bundle?(
    selection: Selection,
    target: string | undefined,
    credentials: Record<string, string>,
  ): Promise<Bundle>;
}
