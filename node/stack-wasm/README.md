# @open-lakehouse/stack-wasm

The in-browser planner for the environment editor: a thin TypeScript wrapper over
the `stack-topology-wasm` Rust crate (the pure `olai-stack-topology` planner
compiled to `wasm32`). Exposes a `Planner` — `catalog()` and `plan(selection)` —
that the `@open-lakehouse/env-editor` component consumes through its planner seam.

## Entries

- **`.`** (`index.ts`) — `createWasmPlanner()`, backed by the real wasm module. It
  imports the wasm-bindgen artifact via the bare specifier
  `"stack-topology-wasm-pkg"` (typed by `pkg.d.ts`), which the app's Vite config
  aliases to the built `pkg/` when `VITE_ENABLE_WASM=true`.
- **`./stub`** (`stub.ts`) — a fixture-backed `createWasmPlanner()` with no wasm
  dependency. **Default builds and Storybook alias the whole package here**, so a
  plain `bun install && bun run dev` works before the wasm artifact is built.
- **`./types`** (`types.ts`) — the DTO + `Planner` TypeScript types (the single
  source of truth, hand-kept in sync with `crates/stack-topology-wasm/src/dto.rs`).

## Building the wasm artifact

```bash
just build-stack-wasm      # cargo build --target wasm32 + wasm-bindgen --target web
VITE_ENABLE_WASM=true just ui-dev   # run the app against the real planner
```

The output lands in `pkg/` (gitignored). Fresh clones run the stub until built.

## Regenerating the fixtures

`src/fixtures.ts` is captured from the real planner so the stub and Storybook
render true output. To regenerate, run a small Rust program that calls
`stack_topology_wasm::dto::{catalog_dto, plan_result}` and serializes the results
to JSON, then paste them back into `fixtures.ts` (do not hand-edit).
