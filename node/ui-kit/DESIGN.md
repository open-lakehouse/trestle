# Design token contract — `@open-lakehouse/ui-kit`

This is the shared design contract for trestle's web UI (the environment editor
and its demo app). It follows the
[`design.md`](https://github.com/google-labs-code/design.md) convention: a single
place that fixes the *contract*, not the concrete look. It is intentionally the
same contract the sibling Open Lakehouse apps (mangrove's Unity Catalog UI,
headwaters' lineage UI) align on, so a component themed here drops into any of
them unchanged.

The primitives in this package are **headless with respect to theme**. They emit
semantic Tailwind utilities (`bg-background`, `text-foreground`, `border-border`,
…) and own **no color values, no light/dark palette, and no Tailwind config**.
Each host application supplies the actual values as CSS variables. Aligning on the
variable *names and meanings* below is what keeps separate apps visually coherent
while staying independently themeable.

## How theming is injected

1. The host defines the variables below on `:root` (light) and `.dark` (dark) in
   its own global stylesheet.
2. The host maps them to Tailwind color tokens via `@theme inline`
   (`--color-background: hsl(var(--background));` …).
3. The host adds a `@source` glob so Tailwind scans this package's source and
   generates the utilities the primitives reference
   (`@source "../../ui-kit/src";`).

The reference implementation of steps 1–3 is `node/env-app/src/globals.css`. A
host may pick entirely different values — only the variable names are the contract.

## The variables (HSL components: `H S% L%`)

Every consumer MUST define all of these on both `:root` and `.dark`.

### Surfaces & text
| Variable | Meaning |
| --- | --- |
| `--background` / `--foreground` | App canvas + primary text on it |
| `--card` / `--card-foreground` | Raised surface (panels, cards, graph nodes) + its text |
| `--popover` / `--popover-foreground` | Floating surface (menus, tooltips) + its text |
| `--muted` / `--muted-foreground` | Subdued fill + secondary/label text |

### Interaction
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
| `--radius` | Base corner radius (length, e.g. `0.5rem` — not an HSL triple) |

### Status accents
| Variable | Meaning |
| --- | --- |
| `--status-planned` | Neutral / not started |
| `--status-ready` | Ready / warning-amber |
| `--status-done` | Success |
| `--status-in-progress` | Active (usually tracks `--primary`) |
| `--status-blocked` | Error / blocked |

### Typography (font stacks, not HSL)
| Variable | Meaning |
| --- | --- |
| `--font-sans` | UI sans-serif stack |
| `--font-mono` | Monospace stack (rendered artifacts, identifiers, ports) |

## Rules

- **Add a token here before using it in a primitive.** A primitive must never
  reference a `--var` that isn't in this contract, or hosts will render it unset.
- **Values live in hosts, not here.** Do not add a `:root {}` block or a
  Tailwind config to this package.
- **Changing a token's meaning is a contract change** — coordinate across the
  consuming apps (this repo's env-app, mangrove, headwaters) so palettes stay aligned.
