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

/** One node in the dependency diagram (mirrors `GraphNodeDto`). */
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

/** A directed dependency edge: `from` depends on `to` (mirrors `EdgeDto`). */
export interface EdgeDto {
  from: string;
  to: string;
}

/** The dependency graph (mirrors `GraphDto`). */
export interface GraphDto {
  nodes: GraphNodeDto[];
  edges: EdgeDto[];
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

/** The gateway layout (mirrors `GatewayDto`). */
export interface GatewayDto {
  listeners: ListenerDto[];
  clusters: ClusterDto[];
}

/** The rendered stack artifacts (mirrors `ArtifactsDto`). */
export interface ArtifactsDto {
  compose: string;
  envoy: string;
  env: string;
  gitignore: string;
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
  artifacts: ArtifactsDto;
  layout_report: string;
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
