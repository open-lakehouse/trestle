# Component contract — `@open-lakehouse/env-editor`

The environment editor: pick technologies/capabilities → tune per-module knobs →
generate a plan → explore the node-based "markitecture" and the rendered
artifacts. It follows the [`design.md`](https://github.com/google-labs-code/design.md)
convention, scoped to this component's *contract* — its seam, its props, and the
theme tokens it relies on — so it drops into any host that honors the
`@open-lakehouse/ui-kit` token contract.

## The planner seam (headless contract)

The editor is **headless with respect to the engine**: it never imports a wasm
runtime. All planning flows through a `Planner`:

```ts
interface Planner {
  catalog(): Promise<CatalogDto>;
  plan(selection: Selection): Promise<PlanResult>;
}
```

A host injects one of:

1. `createWasmPlanner()` from `@open-lakehouse/stack-wasm` — real in-browser
   planning (the demo app).
2. A fixture `Planner` — Storybook and tests, no wasm.

Injection is either the `planner` prop on `<EnvironmentEditor>` or an ancestor
`<PlannerProvider planner={…}>`. This mirrors the transport-seam pattern the
sibling headwaters lineage UI uses. `env-editor` depends on
`@open-lakehouse/stack-wasm` only for **types** (`import type`), never its
runtime — so it has no wasm coupling and stays reusable.

## Props

| Prop | Meaning |
| --- | --- |
| `planner?` | The `Planner` engine. If omitted, read from an ancestor `PlannerProvider`. |
| `catalog?` | Pre-seed the catalog to skip the async `catalog()` fetch (stories). |
| `colorMode?` | Drives the React Flow canvas colors (`"system"` \| `"light"` \| `"dark"`); defaults to `"system"`. A host with a theme toggle passes its active theme so the canvas tracks the app. |

## Theming contract

The editor consumes the `@open-lakehouse/ui-kit` token contract (see its
`DESIGN.md`) and adds no colors of its own. It specifically relies on:

- **Surfaces/text:** `--card` / `--card-foreground` (nodes, panels, tiles),
  `--muted-foreground` (secondary text), `--background`, `--foreground`.
- **Interaction:** `--primary` (selected node/edge accent, primary action),
  `--border`, `--ring`, `--destructive` (plan-error surface, required marks).
- **Status accents:** `--status-ready` / `--status-in-progress` / `--status-done`
  / `--status-blocked` — the role → color map for module nodes (`graph/roleTheme.ts`).
- **Typography:** `--font-mono` for the rendered artifacts.

Two host responsibilities (not this package's):

1. Import the React Flow stylesheet once: `@import "@xyflow/react/dist/style.css";`.
2. Supply the token values on `:root`/`.dark` and the Tailwind `@source` glob for
   this package's `src`.

## Rules

- **Never import the wasm runtime here** — only `import type` from
  `@open-lakehouse/stack-wasm/types`. The seam is the only coupling to an engine.
- **No hardcoded colors.** New node/status colors go through a ui-kit token, added
  to that contract first.
- **The diagram is presentational.** `MarkitectureCanvas` takes a graph and
  renders it; data (a `PlanResult`) comes from the seam, never a fetch inside the
  canvas.
