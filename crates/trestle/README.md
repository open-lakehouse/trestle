# olai-trestle (`trestle`)

The unified CLI for the [Trestle](https://github.com/open-lakehouse/trestle)
framework. Published as `olai-trestle`; installs a `trestle` binary.

1. **Project scaffolding** — `trestle new <name> --template <name|git-url|path>`
2. **Config bootstrap** — `trestle init` runs a guided interview and writes
   `trestle.yaml` (+ derived `buf.gen.yaml`); `trestle config` reconfigures an
   existing project (both accept the same flags for non-interactive use)
3. **Code generation** — `trestle generate -c trestle.yaml` and
   `trestle enrich-openapi -c trestle.yaml` (see
   [`olai-codegen`](https://crates.io/crates/olai-codegen))

## Install

Prebuilt binaries (no Rust toolchain, no compile) via
[`cargo binstall`](https://github.com/cargo-bins/cargo-binstall):

```bash
cargo binstall olai-trestle
```

This downloads the release archive for your platform and installs the `trestle`
command. Prebuilt targets: Linux x86_64 + aarch64, and macOS on Apple Silicon
(aarch64); other platforms fall back to a source build. Each archive ships with
a `.sha256` checksum and a GitHub build-provenance attestation you can verify:

```bash
gh attestation verify trestle-<target>.tar.gz --repo open-lakehouse/trestle
```

Or build from source:

```bash
cargo install olai-trestle   # or: cargo install --git https://github.com/open-lakehouse/trestle --bin trestle
```

## Scaffolding

| Template | Description |
|----------|-------------|
| `databricks-app-rust` | Axum service + optional React/Vite frontend + Databricks Apps deploy + proto codegen |
| `open-lakehouse-lab` | Envoy + Postgres + SeaweedFS + MLflow + Unity Catalog + Marimo, emulating Databricks URLs |

```bash
trestle list-templates
trestle new my-app --template databricks-app-rust --profile dbx-emulator
trestle new my-lab --template open-lakehouse-lab  --profile lakehouse
```

`--template` auto-detects an embedded name, a git URL, or a local path. Run
`trestle new --help` for all options.

## Profiles and components

A template manifest declares **components** (reusable subtrees) and **profiles**
(named bundles of components). Each component declares typed contributions under
`provides:` — compose includes, Postgres databases, S3 buckets, Envoy
routes/clusters, env vars. trestle merges the `provides:` blocks of every active
component into a single `stack.*` context that the parent template renders into
unified files (`docker/envoy/envoy.yaml`, `.env.example`, …).

```bash
# profile = a named component bundle; --with adds individual components
trestle new my-app --template databricks-app-rust --profile none --with local-stack-postgres
```

## Advanced usage

### Non-interactive scaffolding (CI)

`--non-interactive` skips every prompt. Each variable must then be resolvable
from a `--set` override, a `--values` file, or its manifest default, or the run
fails with a "missing variable" error. `post_init` hooks marked `confirm: true`
are skipped in this mode (they can't be confirmed without a prompt).

```bash
trestle new my-app \
  --template databricks-app-rust \
  --non-interactive \
  --select frontend=react \
  --set project_name=my-app
```

### Selecting components and options

`--select <category>=<value>[,<value>...]` makes explicit category picks instead
of answering the wizard. Repeat the flag for multiple categories; comma-separate
values for a multi-select category. App-private categories are namespaced as
`app.<app-name>.<category>`:

```bash
trestle new my-app -a databricks-app-rust \
  --select storage=object-store \
  --select app.databricks-app-rust.frontend=react,ci
```

Individual variables are set with `--set <name>=<value>` (short form `-D`).

### Values file (`--values <file>`)

For repeatable runs, supply a YAML file with up to three top-level keys:

```yaml
# values.yaml
apps:               # apps to layer on the base (same as repeating --app)
  - databricks-app-rust
selections:         # same shape as --select; value may be a string or a list
  storage: object-store
  app.databricks-app-rust.frontend: [react, ci]
variables:          # variable overrides (same as --set)
  project_name: my-app
  with_ci: true
```

```bash
trestle new my-app --values values.yaml --non-interactive
```

CLI flags take precedence over the file: a `--set` override beats a `variables:`
entry, and `--select`/`--app` are merged with `selections:`/`apps:`.

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
├── components/       # optional local components
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
  - { name: local-stack-postgres, kind: shared, when: "with_x" }
profiles:
  small: [local-stack-postgres]
post_init:
  - { run: "git init && git add -A", confirm: true, description: "Initialise git" }

# component template.yaml
provides:
  compose_includes:   [./docker/compose/x.yaml]
  postgres_databases: []
  envoy_routes:       []
  env_vars:           {}
  extras:             {}   # free-form, surfaced at stack.extras.<key>
```

## Tests

```bash
cargo test -p olai-trestle --test scaffold       # fast: render + structure checks
TRESTLE_TEST_SLOW=1 cargo test -p olai-trestle --test scaffold   # also cargo check + compose config
```

## License

Apache-2.0
