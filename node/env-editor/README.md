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
- compare two revisions, or a revision and the draft (`diffSelections`)

A store without the revision methods, such as browser storage, has drafts only,
and the shell shows nothing revision-related.
`createMemoryWorkspaceStore(doc, { revisions: true })` keeps revisions in
memory.

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
  its revision.
- `environment.actions(envId)`: buttons in that header (Start, Stop). A disabled
  one shows its `reason`.
- `environment.tabs(envId)`: sub-tabs after the editor's Design (Catalog,
  Logs). A disabled tab can't be picked and says why.
- `environment.nodeStatus(envId, moduleId)`: each service's state, a dot on its
  node.
- `environment.subscribe(onChange)`: tells the shell to ask again, e.g. from a
  query cache. Re-rendering the provider with a new object works too.
- `panel`: replaces the bottom panel's static Run output.
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
runtime.

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
