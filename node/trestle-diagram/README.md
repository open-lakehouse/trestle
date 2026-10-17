# @open-lakehouse/trestle-diagram

Embed a trestle environment diagram (the environment editor's canvas) in another
site, such as a blog post or docs page. It is read-only and drawn from a
**graph snapshot**: a small JSON file exported from the editor.

- The diagram is the real canvas (React Flow + ELK layout), not a screenshot.
  Inline it is a fitted picture that never captures page scroll; the expand
  button opens a full-window dialog for panning and zooming.
- A snapshot is frozen. Later planner or catalog changes never redraw an
  existing post.
- The page needs no planner, catalog, or wasm.

## Getting a snapshot

In the environment editor, build the environment and click **Export snapshot**
in the canvas toolbar. This saves `environment.trestle-graph.json`. The same
toolbar exports **SVG** and **PNG** images, for social cards, feeds, or a
no-JS fallback.

## Install

The package is published to npmjs.com, so it installs like any other package:

```bash
bun add @open-lakehouse/trestle-diagram   # needs react / react-dom 19
```

The same versions are also published to GitHub Packages. To install from
there, point the `@open-lakehouse` scope at it in `.npmrc`:

```ini
@open-lakehouse:registry=https://npm.pkg.github.com
//npm.pkg.github.com/:_authToken=${NODE_AUTH_TOKEN}
```

GitHub Packages needs a token even to read. In GitHub Actions, the job's
`GITHUB_TOKEN` works (`permissions: packages: read`) once the consuming
repository has been granted read access in the package's settings. Elsewhere,
use a token with `read:packages`.

## Use

```tsx
import { EnvironmentDiagram } from "@open-lakehouse/trestle-diagram";
import "@open-lakehouse/trestle-diagram/styles.css";
import snapshot from "./lakehouse.trestle-graph.json";

<EnvironmentDiagram snapshot={snapshot} colorMode="dark" />;
```

| Prop | Default | |
| --- | --- | --- |
| `snapshot` | — | The parsed snapshot JSON (validated; a bad or newer-version file renders an error box). |
| `graph` | — | A `GraphDto` to draw directly, instead of a snapshot. |
| `colorMode` | `"system"` | `"light"`, `"dark"`, or `"system"`. Pass your site's active theme. |
| `height` | fitted | A fixed height in px. By default the embed uses the diagram's aspect ratio, clamped to `minHeight`–`maxHeight` (240–640). |
| `legend` | `true` | The connection-kind legend, shown as a row under the diagram. |
| `expandable` | `true` | The expand-to-dialog button. |
| `label` | snapshot `title` | The figure's accessible name. |
| `slots` | — | Renderers for a node's name and role lines (see below). |

The diagram pulls in React Flow and ELK (ELK alone is about 1.6 MB
unminified), so lazy-load it (`React.lazy`) to keep them out of a page's main
bundle.

## Linking nodes to your own pages

A snapshot says what each node is (`id`, the module's `model` ref, its `role`)
but nothing about where your site explains it. `slots` lets the host draw a
node's name and role lines, so they can link to the host's pages and show a
hover card:

```tsx
<EnvironmentDiagram
  snapshot={snapshot}
  slots={{
    name: (node, fallback) => <ConceptLink id={node.model}>{fallback}</ConceptLink>,
    role: (node, fallback) => <ConceptLink id={roleConcept(node.role)}>{fallback}</ConceptLink>,
  }}
/>
```

Each slot gets the node and the text drawn without it; return `undefined` to
keep that text. The lines keep their fixed height and truncate, so render
inline content. Clicks inside a slot don't select or drag the node.

A popover in a slot portals into `useDiagramPortal()`, or `document.body`
when it returns `null` (inline). The expanded view is a modal `<dialog>` that
draws above everything in the body, so there the hook returns a container
inside it. Both are outside the `.trestle-diagram` style scope, so the
popover keeps the host's own styles.

## Styling

`styles.css` is self-contained. It bundles its own Tailwind utilities, preflight,
and React Flow styles, and every rule is scoped under the `.trestle-diagram`
root the component renders. It doesn't need the host's Tailwind or theme, and
it doesn't leak into the host page.

To restyle, override the design tokens (HSL components, as in
`node/env-app/src/globals.css`) on the root:

```css
.trestle-diagram { --primary: 190 100% 35%; }
.trestle-diagram.dark { --primary: 190 90% 60%; }
```

## Snapshot format

```jsonc
{
  "format": "trestle-graph",
  "version": 1,
  "generator": "@open-lakehouse/env-editor",
  "title": "optional",
  "graph": { "nodes": [], "edges": [] } // the planner's GraphDto
}
```

`parseGraphSnapshot` validates the envelope, and `createGraphSnapshot` /
`stringifyGraphSnapshot` produce snapshots. `version` is bumped only when a
graph change would render old snapshots wrongly; this package then refuses
newer versions instead of drawing them incorrectly.

## Development

The code lives in `@open-lakehouse/env-editor` (`src/diagram/`, exported as
`@open-lakehouse/env-editor/diagram`). The editor and the embed share one
canvas, and this package only bundles it:

```bash
cd node
bun run build:diagram   # vite (JS + scoped CSS) → rollup (bundled .d.ts) → dist import check
```

`vite.config.ts` scopes the stylesheet. `scripts/check-dist.mjs` fails the build
if `dist/` imports a package that isn't declared in `dependencies`. Releases
go through release-plz, like the Rust crates. The placeholder crate at
`node/Cargo.toml` holds the version, and `CHANGELOG.md` is generated. The
version in `package.json` stays `0.0.0` and is stamped at publish time. See
"The `@open-lakehouse/trestle-diagram` npm package" in `CONTRIBUTING.md`.
