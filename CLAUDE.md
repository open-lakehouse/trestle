# Repository Guidelines

## Project Structure

Multi-crate Rust workspace:

- `crates/trestle-codegen/` — Proto-driven code generation (REST handlers, clients, bindings, resource registries)
- `crates/trestle-store/` — Generic resource store abstractions (Object, Association, Label, Registry)
- `crates/trestle-derive/` — Proc-macro crate for Object ↔ typed struct conversions
- `crates/trestle-cloud/` — Cloud credential abstraction (AWS, Azure, GCP, Databricks)

## Build & Test

```bash
cargo build              # Build all crates
cargo test --lib --tests # Run unit + integration tests (skip doctests)
cargo test               # Run all tests including doctests
cargo clippy             # Lint
cargo fmt                # Format
```

The `trestle-codegen` crate has doctests disabled (`doctest = false` in `[lib]`)
because prost-generated proto doc comments contain proto-syntax examples that are
not valid Rust.

## Coding Style

- Rust Edition 2024, minimum Rust 1.85
- Standard Rust conventions, 4-space indentation
- `cargo fmt` + `cargo clippy` for formatting and linting

## Key Design Decisions

- **Proto-driven**: All resource types, field roles, and API surface are derived from
  protobuf annotations (`google.api.resource`, `field_behavior`, `debug_redact`).
- **Generic over `Label`**: The store layer is generic over `L: Label`, allowing any
  protobuf-defined resource taxonomy to be plugged in.
- **No UC coupling**: This is a generic framework. Avoid hardcoding references to any
  specific data platform (Unity Catalog, Hive, etc.) in library code. Use generic
  examples in doc comments.

## Commit Guidelines

GPG commit signing is required. **Never run `git commit` directly** — the GPG PIN
prompt needs an interactive terminal. Prepare the commit command for the user to run.
