# Component contract — `@open-lakehouse/env-editor`

The environment editor, built around the node-based "markitecture": add
components from a palette, configure, swap or remove them on the canvas, and
watch the plan update live, with the rendered artifacts one tab away. It follows the [`design.md`](https://github.com/google-labs-code/design.md)
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

## Editing flow

`useEnvironment` owns the working `Selection` (with undo/redo), the catalog and
the live plan: every edit replans after a short debounce, and a failed plan
keeps the last good one on screen next to the error.

- **Canvas** (default tab) — a top toolbar (*Add component* `⌘K` / `/`, undo
  `⌘Z`, redo `⇧⌘Z`, planner status), a hover action bar on each node
  (configure · swap · remove), and a docked inspector for the selected node
  (settings, connections, rendered files, swap/remove). `Delete` removes the
  selected component, `Esc` deselects. The component palette (`⌘K`) adds
  modules and capabilities and removes the ones you added: browsing lists the
  environment first, and a query starting with `remove` narrows to what can
  be removed.
- **Components** — the same selection as a toggle list with inline knobs.
- **Files** — the rendered artifacts.

A component is *explicit* (in `Selection.modules`) or *auto* (pulled in by
another component's requirement or demand, or a selected capability; see
`state/origin.ts`). Only explicit components can be removed; auto ones carry an
"Auto" chip and say what needs them.

A provider's card lists what it provisions, one section per resource kind
(Databases, Buckets) with a row per resource. Each row has its own handle, so a
component's `uses` edge is split per resource and lands on the database or
bucket it uses. Extra resources (`Selection.extra_resources`, pinned to the
provider) are added from the section's `+` or the inspector, validated per
kind, and removable; resources a component demands are not.

`MarkitectureCanvas` is editable only when given `actions` (a `CanvasActions`);
without them it renders the same diagram read-only. The actions reach the nodes
through context, never node `data`, so changing them doesn't re-run the ELK
layout.

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
- **Project logos are monochrome.** `graph/projectIcons.tsx` vendors each logo as
  inline path data filled with `currentColor` (tinted by the role accent), never
  in brand colors. A module without one falls back to its role's lucide icon.
- **The diagram is presentational.** `MarkitectureCanvas` takes a graph and
  renders it; data (a `PlanResult`) comes from the seam, never a fetch inside the
  canvas. Editing is callbacks in (`actions`), never state the canvas owns.
