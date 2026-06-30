<div align="center">

<img src="docs/assets/trestle-logo.png" alt="Trestle" width="120" />

# Trestle

**Build typed, proto-driven data-platform services in Rust — from `.proto` to a running, scaffolded project.**

[![CI](https://github.com/open-lakehouse/trestle/actions/workflows/ci.yml/badge.svg)](https://github.com/open-lakehouse/trestle/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/open-lakehouse/trestle/branch/main/graph/badge.svg)](https://codecov.io/gh/open-lakehouse/trestle)
[![crates.io](https://img.shields.io/crates/v/olai-trestle.svg)](https://crates.io/crates/olai-trestle)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache--2.0-blue.svg)](LICENSE)

</div>

Trestle turns annotated protobuf into production-ready Rust REST APIs, typed
clients, Python (PyO3) and Node.js (NAPI) bindings, and a graph-based resource
store — all derived from proto annotations. Its `trestle new` command scaffolds
full project trees pre-wired for codegen, local lakehouse emulation, and
Databricks Apps deployment.

The pipeline, the crate map, and the design decisions behind it are documented in
[`docs/architecture.md`](docs/architecture.md).

## Crates

| Crate | Description |
|-------|-------------|
| [`olai-trestle`](crates/trestle) | Unified CLI (`trestle` binary): `trestle new` for scaffolding + `trestle generate` / `enrich-openapi` for codegen |
| [`olai-codegen`](crates/olai-codegen) | Proto-driven code generation for REST handlers, clients, resource registries, and language bindings |
| [`olai-store`](crates/olai-store) | Generic, TAO-inspired object and association store with field-role enforcement |
| [`olai-http`](crates/olai-http) | Cloud credential abstraction + HTTP client (AWS, Azure, GCP, Databricks) |
| [`olai-http-wasm`](crates/olai-http-wasm) | Browser/WASM HTTP transport for generated clients |

## Quick start

Install the CLI, then scaffold a project:

```bash
cargo install olai-trestle   # installs the `trestle` binary

# A full Databricks-Apps-ready Rust service + React frontend, with a local
# Postgres/MLflow/Envoy stack that emulates Databricks URLs.
trestle new my-app --template databricks-app-rust --profile dbx-emulator
```

`trestle list-templates` shows the embedded templates;
`trestle new --help` documents every flag. See
[`crates/trestle/README.md`](crates/trestle/README.md) for the full scaffolding
guide and [`crates/olai-codegen/README.md`](crates/olai-codegen/README.md) for the
proto annotations and codegen config.

To use the libraries directly without scaffolding:

```toml
[dependencies]
olai-store = "0.0"
olai-http = "0.0"

[build-dependencies]
olai-codegen = "0.0"
```

## Prerequisites

- **Rust 1.88+** (Edition 2024)
- **[buf](https://buf.build/)** — to compile `.proto` into a descriptor (codegen only)
- **Docker + Docker Compose** *(optional)* — for the local platform stack that
  scaffolded labs/apps bring up

## Build & test

Standard Cargo workspace:

```bash
cargo build               # build all crates
cargo test --lib --tests  # unit + integration tests (skips doctests)
```

For the full development workflow, linting, and how releases work, see
[`CONTRIBUTING.md`](CONTRIBUTING.md).

## License

Apache-2.0. See [LICENSE](LICENSE).
