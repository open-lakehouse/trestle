# `trestle`

The unified CLI for the Trestle framework. Two responsibilities:

1. **Project scaffolding** — `trestle new <name> --template <name|git-url|path>`
2. **Code generation** — `trestle generate -c trestle.yaml`,
   `trestle enrich-openapi -c trestle.yaml` (the former `proto-gen` binary)

## Install

```bash
cargo install --git https://github.com/open-lakehouse/trestle --bin trestle
```

## Embedded templates

| Template | Description |
|----------|-------------|
| `databricks-app-rust` | Axum service + optional React/Vite frontend + Databricks Apps deployment + proto codegen + AI-onboarding docs |
| `open-lakehouse-lab` | Envoy + Postgres + SeaweedFS + MLflow + Unity Catalog + Marimo notebooks, configured to emulate Databricks URLs |

```bash
trestle list-templates
trestle list-components --template databricks-app-rust
trestle new my-app --template databricks-app-rust --profile dbx-emulator
trestle new my-lab --template open-lakehouse-lab  --profile lakehouse
```

Run `trestle new --help` for all options. The CLI auto-detects whether
`--template` is an embedded name, a git URL, or a local path.

## Profiles and components

A template manifest declares **components** (reusable subtrees) and **profiles**
(named bundles of components). For example, `databricks-app-rust` exposes:

```
trestle new my-app --template databricks-app-rust --profile dbx-emulator
                                                  └─ pulls in envoy+postgres+seaweedfs+mlflow+dbx-env

trestle new my-app --template databricks-app-rust --profile none --with local-stack-postgres
                                                  └─ minimal: just postgres
```

Each component declares typed contributions under `provides:`:

```yaml
# _components/local-stack-mlflow/template.yaml
name: local-stack-mlflow
depends_on: [local-stack-postgres, local-stack-seaweedfs, local-stack-envoy]
provides:
  compose_includes:    [./docker/compose/mlflow.yaml]
  postgres_databases:  [mlflow]
  s3_buckets:          [mlflow]
  envoy_clusters:
    - { name: mlflow, host: mlflow, port: 5000 }
  envoy_routes:
    - { prefix: "/api/2.0/mlflow", cluster: mlflow, rewrite: "/mlflow/api/2.0/mlflow" }
  env_vars:
    MLFLOW_TRACKING_URI: "http://localhost:${ENVOY_PORT:-9080}"
```

trestle aggregates the `provides:` blocks of every active component into a
single `stack.*` MiniJinja context, which the parent template uses to render
unified files (`docker/envoy/envoy.yaml`, `docker/postgres/init-databases.sh`,
`.env.example`, ...).

## Authoring a template

```
my-template/
├── template.yaml         # manifest: name, variables, components, profiles, post_init
├── template/             # files copied + minijinja-rendered into the project
│   ├── README.md.tmpl    # .tmpl suffix stripped after rendering
│   └── crates/{{ project_name | kebab_case }}-server/Cargo.toml.tmpl
├── components/           # optional local components
│   └── frontend-react/
│       ├── template.yaml
│       └── template/
└── hooks/                # optional post-init shell scripts
```

Filenames *and* file contents are rendered with [MiniJinja](https://docs.rs/minijinja);
files ending in `.tmpl` get the suffix stripped after rendering. The custom
filter set is:

- `snake_case`, `kebab_case`, `pascal_case`, `camel_case`, `screaming_snake_case`
- `upper_case`, `lower_case`

A `.trestle-ignore` file at the template root (gitignore syntax) excludes
matching paths from copying.

### Manifest reference

```yaml
name: my-template
version: 0.1.0
description: One-line description shown in `trestle list-templates`.

variables:
  - { name: project_name, prompt: "Name", validate: "^[a-z][a-z0-9-]*$" }
  - { name: with_x, type: bool, default: true }
  - { name: choice, type: enum, options: [a, b], default: a }

components:
  - { name: thing,                  kind: local,  path: components/thing, when: "with_x" }
  - { name: local-stack-postgres,   kind: shared, when: "choice == 'a'" }

profiles:
  small:   [local-stack-postgres]
  big:     [local-stack-envoy, local-stack-postgres, local-stack-mlflow]

template_context:
  app_service_name: "{{ project_name | kebab_case }}-server"
  app_port: 8080

post_init:
  - { run: "git init && git add -A", confirm: true,  description: "Initialise git" }
  - { run: "cargo fmt --all",         confirm: false }
```

### Component reference

```yaml
name: local-stack-X
version: 0.1.0
description: One-line description.
depends_on: [other-component]
provides:
  compose_includes:    [./docker/compose/x.yaml]
  postgres_databases:  []
  s3_buckets:          []
  envoy_routes:        []
  envoy_clusters:      []
  env_vars:            {}
  ports:               []
  extras:              {}     # free-form, surfaced at stack.extras.<key>
```

## Tests

```bash
cargo test -p trestle --test scaffold              # fast: renders + checks structure
TRESTLE_TEST_SLOW=1 cargo test -p trestle --test scaffold
                                                  # slow: also runs cargo check
                                                  # and docker compose config on
                                                  # the rendered tree
```

## License

Apache-2.0
