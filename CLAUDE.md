# Repository Guidelines

## Project Structure

Multi-crate Rust workspace:

- `crates/trestle/` — Unified CLI, published as `olai-trestle` (`trestle` binary): `trestle new` / `generate` / `enrich-openapi` + embedded templates at `crates/trestle/templates/`
- `crates/olai-codegen/` — Proto-driven code generation (REST handlers, clients, bindings, resource registries)
- `crates/olai-store/` — Generic resource store abstractions (Object, Association, Label, Registry)
- `crates/olai-http/` — Cloud credential abstraction and HTTP client (AWS, Azure, GCP, Databricks)
- `crates/olai-http-wasm/` — Browser/WASM HTTP transport for generated clients

`trestle new` scaffolds the four-crate project layout described in
`crates/olai-codegen/README.md`.

## Build & Test

```bash
cargo build              # Build all crates
cargo test --lib --tests # Run unit + integration tests (skip doctests)
cargo test               # Run all tests including doctests
cargo clippy             # Lint
cargo fmt                # Format
```

The `olai-codegen` crate has doctests disabled (`doctest = false` in `[lib]`)
because prost-generated proto doc comments contain proto-syntax examples that are
not valid Rust.

## Coding Style

- Rust Edition 2024, MSRV 1.88 (the workspace `rust-version`; read it, don't assume)
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

## Credential redaction (olai-http)

Credential structs hold long-lived secrets and must never leak them through
`Debug` (easily triggered via `tracing`, `{:?}`, or panic messages). When adding
a credential type: do **not** `#[derive(Debug)]`; hand-write `impl fmt::Debug`
rendering `<redacted>` for every secret-bearing field (use `Some("<redacted>")`
/ `None` for `Option`s so presence is still observable). See `AwsCredential` in
`crates/olai-http/src/aws/credential.rs` for the reference.

## Commit Guidelines

Commits are GPG-signed; the message contract and signing flow are machine-wide
(`~/.claude/CLAUDE.md`). Use the `/commit` skill (`.claude/skills/commit/SKILL.md`):
commit **unsigned** as you go, push and open the PR without waiting on signing,
then **sign once at the end** with a single combined sign + `--force-with-lease`
push. Conventional-commit scopes are crate names; prefer small, per-crate commits.
