# olai-trestle (`trestle`)

The unified CLI for the [Trestle](https://github.com/open-lakehouse/trestle)
framework. Published as `olai-trestle`; installs a `trestle` binary.

1. **Project scaffolding** — `trestle new <name> --template <name|git-url|path>`
2. **Code generation** — `trestle generate -c trestle.yaml` and
   `trestle enrich-openapi -c trestle.yaml` (see
   [`olai-codegen`](https://crates.io/crates/olai-codegen))

## Install

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
