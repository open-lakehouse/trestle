# olai-trestle (`trestle`)

The unified CLI for the [Trestle](https://github.com/open-lakehouse/trestle)
framework. Published as `olai-trestle`; installs a `trestle` binary.

1. **Project scaffolding**: `trestle new <name>` renders a base template plus
   optional apps (`--app`).
2. **Config bootstrap**: `trestle init` runs a guided interview and writes
   `trestle.yaml` (+ derived `buf.gen.yaml`); `trestle config` reconfigures an
   existing project (both accept the same flags for non-interactive use).
3. **Code generation**: `trestle generate -c trestle.yaml` and
   `trestle enrich-openapi -c trestle.yaml` (see
   [`olai-codegen`](https://crates.io/crates/olai-codegen)).
4. **Dev environments**: `trestle env new <name>` renders a Docker Compose stack
   from selected modules; `trestle workspace` runs several of them side by side.

## Install

Prebuilt binaries (no Rust toolchain, no compile) via
[`cargo binstall`](https://github.com/cargo-bins/cargo-binstall):

```bash
cargo binstall olai-trestle
```

This downloads the release archive for your platform and installs the `trestle`
command. Prebuilt targets: Linux (x86_64 and aarch64) and macOS (Apple Silicon
and Intel). Other platforms fall back to a source build. Each archive ships with
a `.sha256` checksum and a GitHub build-provenance attestation you can verify:

```bash
gh attestation verify trestle-<target>.tar.gz --repo open-lakehouse/trestle
```

Or build from source:

```bash
cargo install olai-trestle   # or: cargo install --git https://github.com/open-lakehouse/trestle --bin trestle
```

## Scaffolding

| Template | Kind | Description |
|----------|------|-------------|
| `lakehouse` | base (default) | Project skeleton for an open-lakehouse app; the composed dev environment is created separately with `trestle env`. |
| `databricks-app-rust` | app (`--app`) | Service + optional frontend + proto codegen; deployable to Databricks Apps. |

```bash
trestle list-templates
trestle new my-app --app databricks-app-rust
cd my-app
trestle env new my-app --out-dir env   # the composed dev environment
```

`--template` picks the base and accepts an embedded name, a git URL, or a local
path. `--app` layers an app on top and can be repeated; it takes the same forms.
Run `trestle new --help` for all options.

## Categories and components

Each app declares **categories**: named choices with explicit `options:`. Pick
them with `--select app.<app>.<category>=<value>`, or answer the wizard. A
category you don't pick takes its default. `databricks-app-rust` declares:

| Category | Options | Default |
|----------|---------|---------|
| `frontend` | `react`, `none` | `react` |
| `ci` | `github`, `none` | `github` |
| `connect` | `on`, `none` | `none` (`on` forces the buffa runtime) |

Templates also declare **components** (`components:`), reusable subtrees that
are enabled by a `when:` expression. Each component declares typed contributions
under `provides:`: compose includes, Postgres databases, Envoy routes, env vars
and free-form `extras`. trestle merges the `provides:` blocks of every active
component into one `stack.*` context that the parent templates render.

`--profile` and `--with` are legacy shims over a base's `profiles:` block and
component names. `--select` is the way to choose.

## Advanced usage

### Non-interactive scaffolding (CI)

`--non-interactive` skips every prompt. Each variable must then be resolvable
from a `--set` override, a `--values` file, or its manifest default, or the run
fails with a "missing variable" error. `post_init` hooks marked `confirm: true`
are skipped in this mode (they can't be confirmed without a prompt).

```bash
trestle new my-app \
  --app databricks-app-rust \
  --non-interactive \
  --select app.databricks-app-rust.frontend=react \
  --set gh_owner=acme
```

### Selecting components and options

`--select <category>=<value>[,<value>...]` makes explicit category picks instead
of answering the wizard. Repeat the flag for multiple categories; comma-separate
values for a multi-select category. App-private categories are namespaced as
`app.<app-name>.<category>`. Values must be among the category's options.

Individual variables are set with `--set <name>=<value>` (short form `-D`).

### Values file (`--values <file>`)

For repeatable runs, supply a YAML file with up to three top-level keys:

```yaml
# values.yaml
apps:               # apps to layer on the base (same as repeating --app)
  - databricks-app-rust
selections:         # same shape as --select; value may be a string or a list
  app.databricks-app-rust.frontend: react
  app.databricks-app-rust.ci: github
variables:          # variable overrides (same as --set); only declared variables apply
  gh_owner: acme
  runtime: buffa
```

```bash
trestle new my-app --values values.yaml --non-interactive
```

CLI flags win over the file. A `--set` overrides the same `variables:` key, a
`--select` replaces the same category in `selections:`, and `--app` adds to
`apps:`.

## Troubleshooting

- **`git ... failed` when using a git template.** Git templates are cloned with
  the system `git` binary, so `git` must be on `PATH`. Install it (or use an
  embedded name / local path with `--template`) and retry.
- **"missing variable" under `--non-interactive`.** A variable had no override,
  no `--values` entry, and no manifest default. Supply it with `--set` or in the
  `--values` file.
- **Output directory already exists and is non-empty.** Pass `--force` to render
  into it anyway; existing files at generated paths are overwritten, others are
  left untouched.

## Authoring a template

```text
my-template/
├── template.yaml     # manifest: variables, components, profiles, post_init
├── template/         # files copied + MiniJinja-rendered (.tmpl suffix stripped)
├── components/       # local components (one directory each, with a template.yaml)
└── hooks/            # optional post-init scripts
```

Filenames *and* contents render with [MiniJinja](https://docs.rs/minijinja),
with extra case filters (`snake_case`, `kebab_case`, `pascal_case`, `camel_case`,
`screaming_snake_case`, `upper_case`, `lower_case`). A `.trestle-ignore` file
(gitignore syntax) at the template root excludes paths from copying.

```yaml
# template.yaml
name: my-template
variables:
  - { name: project_name, prompt: "Name", validate: "^[a-z][a-z0-9-]*$" }
  - { name: with_x, type: bool, default: true }
components:
  - { name: x, kind: local, path: components/x, when: "with_x" }
profiles:
  small: [x]
post_init:
  - { run: "git init && git add -A", confirm: true, description: "Initialise git" }

# component template.yaml (components/x/template.yaml)
name: x
provides:
  compose_includes: [./docker/compose/x.yaml]
  postgres_databases: []
  envoy_routes: []
  env_vars: {}
  extras: {}   # surfaced as stack.extras["x__<key>"]
```

## Tests

```bash
cargo test -p olai-trestle --test scaffold       # fast: render + structure checks
TRESTLE_TEST_SLOW=1 cargo test -p olai-trestle --test scaffold   # also cargo check + compose config
```

## License

Apache-2.0
