# `@open-lakehouse/ui-kit`

The shared presentational kit for the Open Lakehouse web UIs: the shadcn/Radix
primitives (`Button`, `Dialog`, `Select`, `Tooltip`, …), the `cn` class-merge
helper, and the `ThemeProvider` / `useTheme` theme context. Trestle's
env-editor, mangrove's Unity Catalog UI, headwaters' lineage UI and the desktop
shell all build on it, so the primitives live in one place instead of being
copied per repo.

## Install

```bash
bun add @open-lakehouse/ui-kit   # or npm / pnpm
```

Published to npmjs.com and GitHub Packages. `react` and `react-dom` (19) are
peers.

## Public surface

Import from the barrel, `@open-lakehouse/ui-kit`:

- Primitives: `Badge`, `Button`, `Card*`, `Dialog*`, `DropdownMenu*`, `Input`,
  `Label`, `ResizeHandle` / `usePaneSize`, `Select*`, `Separator`, `Sheet*`,
  `Switch`, `Tabs*`, `Textarea`, `Toaster`, `Tooltip*` (plus the `*Variants` and
  `*Props` for `Badge` / `Button`).
- Helpers: `cn`.
- Theme: `ThemeProvider`, `useTheme`.

## Tailwind

This package ships Tailwind utility classes but owns no Tailwind config and no
theme values. The consuming app supplies the design tokens and points a
`@source` glob at this package so Tailwind generates the utilities. The token
contract every consumer must satisfy, and the `@source` lines, are in
[`DESIGN.md`](./DESIGN.md).

## Distribution

`bun run build` emits ESM + declarations to `dist/`, and `exports` point there.
Inside trestle's `node/` workspace, consumers resolve the `source` export
condition to `src/` instead (`customConditions` in `tsconfig.base.json`,
`resolve.conditions` in the Vite configs), so local development needs no build
step. Releases are cut by release-plz through the placeholder crate in
`Cargo.toml`; see [`node/README.md`](../README.md).
