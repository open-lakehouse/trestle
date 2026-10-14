// `@open-lakehouse/env-diagram`: the published, read-only environment diagram.
// Everything is implemented in `@open-lakehouse/env-editor/diagram` (the editor
// and the embed share one canvas); this package bundles it so a host needs no
// workspace sources, wasm, or planner. Styles: `@open-lakehouse/env-diagram/styles.css`.

export * from "@open-lakehouse/env-editor/diagram";
