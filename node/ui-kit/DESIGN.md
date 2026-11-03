# Design token contract: `@open-lakehouse/ui-kit`

This is the shared design contract for every Open Lakehouse web UI: trestle's
environment editor, mangrove's Unity Catalog UI, headwaters' lineage UI and the
desktop shell. It follows the
[`design.md`](https://github.com/google-labs-code/design.md) convention. It is
the one place that fixes the *contract*, but not the concrete look, so a
component themed here drops into any of those apps unchanged.

The primitives in this package are **headless with respect to theme**. They emit
semantic Tailwind utilities (`bg-background`, `text-foreground`, `border-border`,
…) and own **no color values, no light/dark palette, and no Tailwind config**.
Each host application supplies the actual values as CSS variables. Separate apps
stay visually coherent because they agree on the variable *names and meanings*
below, and each one can still pick its own values.

## How theming is injected

1. The host defines the variables below on `:root` (light) and `.dark` (dark) in
   its own global stylesheet.
2. The host maps them to Tailwind color tokens via `@theme inline`
   (`--color-background: hsl(var(--background));` …).
3. The host adds a `@source` glob so Tailwind scans this package and generates
   the utilities the primitives reference. When the package is installed from the
   registry, point the glob at the build. Inside a workspace that links it, point
   it at the source:

   ```css
   @source "../node_modules/@open-lakehouse/ui-kit/dist"; /* installed */
   @source "../../ui-kit/src";                            /* workspace */
   ```

   Do the same for every other `@open-lakehouse/*` UI package the host renders,
   including ones it only renders indirectly.

The reference implementation of steps 1–3 is trestle's
`node/env-app/src/globals.css`. A host may pick entirely different values. Only
the variable names are the contract.

## The variables (HSL components: `H S% L%`)

Every consumer MUST define the required groups on both `:root` and `.dark`. An
optional group is needed only by the views that use it.

### Surfaces & text (required)
| Variable | Meaning |
| --- | --- |
| `--background` / `--foreground` | App canvas + primary text on it |
| `--card` / `--card-foreground` | Raised surface (panels, cards, graph nodes) + its text |
| `--popover` / `--popover-foreground` | Floating surface (menus, tooltips) + its text |
| `--muted` / `--muted-foreground` | Subdued fill + secondary/label text |

### Interaction (required)
| Variable | Meaning |
| --- | --- |
| `--primary` / `--primary-foreground` | Primary action fill + text on it (also the graph's selected/incident accent) |
| `--secondary` / `--secondary-foreground` | Secondary action fill + text on it |
| `--accent` / `--accent-foreground` | Hover/active accent fill + text on it |
| `--destructive` / `--destructive-foreground` | Danger action fill + text on it |
| `--success` / `--success-foreground` | Success fill + text on it (used by the `success` badge) |
| `--border` | Hairline borders (also the default `border-color`) |
| `--input` | Form control border |
| `--ring` | Focus ring |
| `--radius` | Base corner radius (a length such as `0.5rem`, not an HSL triple) |

### Status accents (required)
| Variable | Meaning |
| --- | --- |
| `--status-planned` | Neutral / not started |
| `--status-ready` | Ready / warning-amber |
| `--status-done` | Success |
| `--status-in-progress` | Active (usually tracks `--primary`) |
| `--status-blocked` | Error / blocked |

### IDE chrome (optional; IDE-style shells such as env-editor's `WorkspaceShell`)
| Variable | Meaning |
| --- | --- |
| `--sidebar` / `--sidebar-foreground` | Sidebar and tab-strip surface + text |
| `--sidebar-accent` | Selected / hovered row in the sidebar |
| `--activitybar` | The icon rail left of the sidebar |
| `--statusbar` / `--statusbar-foreground` | The status bar along the bottom + text on it |

### Delta-log action slots (optional; mangrove's `log-query` views)
| Variable | Meaning |
| --- | --- |
| `--action-add` / `--action-remove` | add / remove actions (reuse the good / critical hues) |
| `--action-metadata` / `--action-protocol` / `--action-txn` / `--action-domain` | the neutral action kinds, as distinct categorical hues |

These views always pair a hue with an icon and label.

### Typography (font stacks, not HSL; required)
| Variable | Meaning |
| --- | --- |
| `--font-sans` | UI sans-serif stack |
| `--font-mono` | Monospace stack (code, identifiers, ports, data cells) |

## Rules

- **Add a token here before using it in a primitive.** A primitive must never
  reference a `--var` that isn't in this contract, or hosts will render it unset.
- **Values live in hosts, not here.** Do not add a `:root {}` block or a
  Tailwind config to this package.
- **Changing a token's meaning is a contract change.** Coordinate it across the
  consuming apps (trestle's env-app, mangrove, headwaters, the desktop shell) so
  palettes stay aligned.
