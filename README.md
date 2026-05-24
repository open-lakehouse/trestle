# Trestle

A framework for building data-platform services from protobuf definitions, with
a CLI that scaffolds full Databricks-ready projects (and local lakehouse labs)
out of the box.

Trestle turns compiled protobuf descriptors into production-ready Rust REST APIs,
typed clients, Python (PyO3) and Node.js (NAPI) bindings, and a graph-based
resource store — all driven by proto annotations — and a `trestle new`
sub-command that produces full project trees pre-wired for proto-driven codegen,
local Databricks emulation, and Databricks Apps deployment.

## Crates

| Crate | Description |
|-------|-------------|
| [`trestle`](crates/trestle) | Unified CLI: `trestle new` for scaffolding + `trestle generate`/`enrich-openapi` for codegen |
| [`olai-codegen`](crates/olai-codegen) | Proto-driven code generation for REST handlers, clients, resource registries, and language bindings |
| [`olai-store`](crates/olai-store) | Generic, TAO-inspired object and association store with field-role enforcement |
| [`olai-http`](crates/olai-http) | HTTP client based on reqwest with built-in authorization for many clouds. |

## How it works

```text
.proto files (with google.api.resource, field_behavior, debug_redact annotations)
    │
    ▼  buf build
descriptor.bin
    │
    ▼  olai-codegen (olai-codegen)
    ├── Axum handler traits + route wiring
    ├── HTTP client structs
    ├── PyO3 bindings + .pyi typings
    ├── NAPI bindings + TypeScript client
    ├── ObjectLabel enum (impl Label)
    └── RESOURCE_DESCRIPTORS registry (field roles from annotations)
    │
    ▼  olai-store
    ├── ObjectStore<L> / AssociationStore<L>  — async CRUD + graph ops
    ├── ManagedObjectStore                    — field-role enforcement
    └── SecretManager                         — encrypted sensitive fields
```

## Quick start

### Scaffold a new project

```bash
cargo install --git https://github.com/open-lakehouse/trestle --bin trestle

# A full Databricks-Apps-ready Rust service + React frontend, with a local
# Postgres/MLflow/Envoy stack that emulates Databricks URLs.
trestle new my-app --template databricks-app-rust --profile dbx-emulator

# A no-app open lakehouse playground (Envoy + Postgres + MLflow + Unity
# Catalog + SeaweedFS + Marimo notebooks) for prototyping and demos.
trestle new my-lab --template open-lakehouse-lab --profile lakehouse
```

`trestle list-templates` and `trestle list-components --template <name>` show
what's available out of the box.

### Use the libraries directly

```toml
[dependencies]
olai-store = "0.1"
olai-http = "0.1"

[build-dependencies]
olai-codegen = "0.1"
```

See each crate's README for detailed usage. The recommended directory layout
that `crates/olai-codegen/README.md` describes is the layout `trestle new
databricks-app-rust` produces.

## Embedded templates

| Template | What it builds |
|----------|----------------|
| `databricks-app-rust` | Axum service + optional React/Vite frontend + Asset-Bundle deploy + AI-onboarding docs |
| `open-lakehouse-lab` | Envoy gateway + Postgres + MLflow + Unity Catalog + SeaweedFS + Marimo notebooks |

Both templates draw from a shared library of platform-stack components at
[`crates/trestle/templates/_components/`](crates/trestle/templates/_components/):
`local-stack-envoy`, `local-stack-postgres`, `local-stack-seaweedfs`,
`local-stack-mlflow`, `local-stack-unity-catalog`, `local-stack-notebooks`,
`local-stack-jaeger`, `databricks-emulator-env`. See
[`crates/trestle/README.md`](crates/trestle/README.md) for the authoring guide.

## Requirements

- Rust 1.85+ (Edition 2024)
- [buf](https://buf.build/) for proto compilation (codegen only)
- (Optional) Docker + Docker Compose for the local platform stack

## License

Apache-2.0
