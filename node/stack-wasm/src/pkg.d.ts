// Ambient typing for the "stack-topology-wasm-pkg" bare specifier — the
// wasm-bindgen output of `just build-stack-wasm`
// (node/stack-wasm/pkg/stack_topology_wasm.js, gitignored). `index.ts` imports
// the BARE name so this package type-checks whether or not the artifact exists;
// node/env-app/vite.config.ts aliases the name to the real file in wasm-enabled
// builds (default builds alias the whole package to ./stub.ts, so the artifact
// is never resolved).
//
// Keep the shapes in sync with crates/stack-topology-wasm/src/bindings.rs (the
// generated pkg/stack_topology_wasm.d.ts is the source of truth).
declare module "stack-topology-wasm-pkg" {
  /** wasm-bindgen `--target web` default export: fetch + instantiate the
   *  `.wasm`. Resolves once the module is ready. */
  export default function initWasm(
    module_or_path?: string | URL | Request | BufferSource | WebAssembly.Module,
  ): Promise<unknown>;

  /** Named export matching `#[wasm_bindgen] pub fn init()` — installs the
   *  readable-panic console hook (call once after the default init resolves). */
  export function init(): void;

  /** The selectable catalog as a plain JS object (see `CatalogDto`). */
  export function catalog(): unknown;

  /** Plan a JSON-serialized `Selection` for a deploy target (Docker Compose
   *  when omitted), under a JSON `PlanCtx` (defaults when omitted); returns a `PlanResult` object or throws a string message on
   *  a deserialization / plan error. */
  export function plan(
    selectionJson: string,
    target?: string | null,
    contextJson?: string | null,
  ): unknown;

  /** Plan a saved environment manifest (`env.toml` text) with its whole
   *  context; returns a `PlanResult` object or throws a string message. */
  export function planManifest(manifestToml: string): unknown;

  /** Render every file of a selection's environment (see `Bundle`), with the
   *  in-stack credentials from a JSON object; throws a string message on a
   *  refused credential or a plan error. */
  export function bundle(
    selectionJson: string,
    target: string | null | undefined,
    credentialsJson: string,
    contextJson?: string | null,
  ): unknown;
}
