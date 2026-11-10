# `@open-lakehouse/env-editor`

The trestle environment editor: `EnvironmentEditor`, the node-based editor for
one environment (pick modules, tune their settings, see the planned diagram and
the rendered files), and `WorkspaceShell`, an IDE-style shell for a workspace of
environments. Trestle's web app and the desktop shell both embed it.

```bash
bun add @open-lakehouse/env-editor   # or npm / pnpm
```

`react` and `react-dom` (19) are peers. The planner's types come from
`@open-lakehouse/stack-wasm`, a dependency of this package, and are re-exported
from here.

## Seams

The editor runs no engine and owns no storage itself. The host injects both:

- **`Planner`** (`<PlannerProvider planner>`): `catalog()`, `plan()` and
  optionally `bundle()`. Every method is async, so the planner can be the
  in-browser wasm build, a native planner behind the desktop's IPC, or the
  fixture planner from `@open-lakehouse/stack-wasm/stub`.
- **`WorkspaceStore`** (`<WorkspaceProvider store>`): `load`, `save`, `saveEnv`
  and an optional `subscribe`. The package ships
  `createLocalStorageWorkspaceStore()` (browser storage) and
  `createMemoryWorkspaceStore()` (tests and stories).

```tsx
<PlannerProvider planner={planner}>
  <WorkspaceProvider store={store}>
    <WorkspaceShell />
  </WorkspaceProvider>
</PlannerProvider>
```

### Revisions

A store may also keep revisions: numbered, immutable snapshots of an
environment, like trestle's shared state store (`olai-stack-state`). It adds all
three optional methods: `listRevisions(envId)`, `loadRevision(envId, n)` and
`commit(envId, message?)`. `supportsRevisions(store)` checks for them.

The editor always edits the **draft** (`EnvDoc.selection`). Committing it makes
the next revision and moves `EnvDoc.head`. With such a store the shell adds a
History view, where you can:

- commit the draft
- open a revision read-only (its diagram and selection), and restore it to the
  draft
- compare two revisions, or a revision and the draft (`diffSelections`, then
  each rendered file that differs, `diffFiles`)

Each environment's tab also gets a Changes view under Configure: the draft
against its last revision.

A store without the revision methods, such as browser storage, has drafts only,
and the shell shows nothing revision-related.
`createMemoryWorkspaceStore(doc, { revisions: true })` keeps revisions in
memory.

### Environment tabs

The explorer lists one row per environment, and picking one opens its tab.
Every environment tab has a header (name, state, details, actions) and two
layers, switched in the header, with `⌘1` / `⌘2`, or from `⌘K`:

- **Configure**: the editor's Diagram, Files (the rendered files) and Changes
  (with a store that keeps revisions), then a host's configure-layer tabs.
  Opening a file from the canvas or the inspector lands on Files at that file.
- **Apps**: a host's tabs. While the host reports the environment as anything
  but running (or degraded), Apps shows a single prompt to start it. Without
  apps-layer tabs the layer and its switch are hidden.

Revision and comparison tabs still open as tabs of their own.

### Host contributions

A host that runs environments (the desktop shell) can put its run state and
views inside the shell with `<ContributionsProvider contributions>`. The editor
draws what it is given and knows nothing about how anything runs. Everything is
optional, and with no contributions the shell is unchanged:

- `sideViews`: extra activity-bar views (`align: "end"` puts one at the
  bottom).
- `activityActions`: activity-bar buttons that run instead of opening a sidebar
  view, e.g. a host's settings in a dialog over the workspace. They are listed
  in the ⌘K palette as well.
- `environment.status(envId)`: a run-state dot and placement (`local slot 0`, a
  remote branch) in the tree, and the header over the environment's tab, with
  its revision. `details` adds facts to that header (`{ label, value, copy?,
  title? }`, e.g. a remote target or the gateway URL with a copy button); they
  replace the editor's own gateway detail.
- `environment.actions(envId)`: buttons in that header (Start, Stop). A disabled
  one shows its `reason`.
- `environment.tabs(envId)`: sub-tabs. `layer: "apps"` (the default) puts one
  in Apps (Catalog, Logs), `layer: "configure"` after the editor's Configure
  tabs (Policies). A disabled tab can't be picked and says why.
- `environment.nodeStatus(envId, moduleId)`: each service's state, a dot on its
  node.
- `environment.subscribe(onChange)`: tells the shell to ask again, e.g. from a
  query cache. Re-rendering the provider with a new object works too.
- `panel`: replaces the bottom panel's static Run output.
- `fileViewer({ envId, path, read })` and `diffViewer({ envId, path, original,
  modified })`: render a file under Files, and each file's change in Changes
  and comparisons, e.g. with Monaco. The editor's own are the Shiki
  `CodeViewer` and a unified line diff (`DiffViewer`).
- `commands`: ⌘K palette commands under their own sections (`Environment`,
  `App`), each with an optional `shortcut`, `hint` (e.g. the `trestle` CLI
  equivalent) and `enabled`/`reason`. Their bindings take precedence over the
  editor's (`EDITOR_SHORTCUTS`); see `DESIGN.md` for the keymap rules.

```tsx
<WorkspaceProvider store={store}>
  <ContributionsProvider contributions={contributions}>
    <WorkspaceShell />
  </ContributionsProvider>
</WorkspaceProvider>
```

The `WorkspaceShell/WithContributions` story drives every seam from a fake
runtime, and `WorkspaceShell/WithViewerSeams` swaps in host viewers.

### Running a module as a Neon Function

A module that can run as a function (its catalog entry sets `function`, the
slug it deploys under, as `unity-catalog-rs` and headwaters do) gets a "Where it
runs" choice in the inspector. **Local container** is the default and leaves
`selection.hosted[id]` unset. **Neon Function** writes
`{ vendor: "neon-function", connection: { kind: "function" } }` there, with an
optional `url`. On the canvas the module then sits in the Neon cloud frame, and
nothing of it runs locally. The editor only edits the selection: the host binds
the Neon branch the function runs on (e.g. `trestle up --neon-branch`). The
`EnvironmentEditor/NeonFunctions` story shows both modules as functions.

While a module runs as a function its settings are hidden: they configure the
local container, and the function runs with the configuration it was deployed
with.

### Choosing an implementation

Modules that serve the same API (the catalog's `api`, e.g. `unity-catalog` for
both `unity-catalog-oss` and `unity-catalog-rs`) are alternatives for one
component, one per environment. The inspector shows an "Implementation" choice
between them, which swaps the module (its settings reset; they belong to the
implementation). The component palette offers the other one as a switch, like
a second object store. Where it runs is a separate choice, offered only by an
implementation that can run as a function.

## Styling

The components use Tailwind utilities and the `@open-lakehouse/ui-kit` token
contract (see its `DESIGN.md`), and import no CSS. The host:

- defines the tokens
- imports React Flow's stylesheet (`@xyflow/react/dist/style.css`)
- points Tailwind at this package and ui-kit:

```css
@source "../node_modules/@open-lakehouse/env-editor/dist";
@source "../node_modules/@open-lakehouse/ui-kit/dist";
```

The component contract (props, the planner seam, tokens) is in
[`DESIGN.md`](./DESIGN.md). The read-only diagram embed is
`@open-lakehouse/env-editor/diagram`, also bundled standalone as
`@open-lakehouse/trestle-diagram`.
