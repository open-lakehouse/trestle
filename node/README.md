# Environment editor web workspace

A Bun workspace for the environment-editor UI: a headless React component that
drives the `olai-stack-topology` planner (compiled to WASM) — pick technologies
or capabilities, tune each module's knobs, generate the environment, and explore
the node-based "markitecture" and rendered artifacts.

## Packages

| Package | Role |
| --- | --- |
| [`@open-lakehouse/ui-kit`](./ui-kit) | Shared headless Radix + CVA primitives + the design-token contract (`DESIGN.md`). |
| [`@open-lakehouse/stack-wasm`](./stack-wasm) | The in-browser planner: a TS wrapper over the `stack-topology-wasm` crate, plus a fixture stub. |
| [`@open-lakehouse/env-editor`](./env-editor) | The headless editor component (selection → knobs → diagram → artifacts) and its planner seam (`DESIGN.md`). |
| [`@open-lakehouse/env-editor-app`](./env-app) | The Vite SPA shell that mounts the editor and supplies the theme. |

The layering mirrors the sibling repos: mangrove (the shadcn-pattern + Tailwind v4
design system) and headwaters (the `@xyflow/react` + `elkjs` graph canvas).

## Quick start

```bash
# From the repo root:
just setup-wasm      # once: wasm32 target + matching wasm-bindgen CLI
just ui-install      # bun install

# Iterate on the UI against the fixture planner (no wasm build):
just ui-dev

# Run against the real in-browser planner (builds the wasm first):
just ui-dev-wasm     # → http://localhost:3020

# Lint + typecheck:
just ui-check
```

## How the WASM is wired

`stack-topology-wasm` (a root Cargo workspace crate) compiles to `wasm32` via
`wasm-bindgen --target web`; the output lands in `stack-wasm/pkg/` (gitignored).
`@open-lakehouse/stack-wasm` imports it through the bare specifier
`stack-topology-wasm-pkg`, which the app's `vite.config.ts` aliases to the built
artifact **only when `VITE_ENABLE_WASM=true`**. Default builds alias the package
to its fixture-backed `./stub`, so a fresh clone runs with no wasm build — and
Storybook/tests never need it. This is the same opt-in-wasm pattern mangrove's
`query-wasm` uses.
