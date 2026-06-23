# Repository Guidelines

## Project Structure

Multi-crate Rust workspace:

- `crates/trestle/` — Unified CLI (`trestle new`, `trestle generate`, `trestle enrich-openapi`) + embedded templates and shared component library at `crates/trestle/templates/`
- `crates/olai-codegen/` — Proto-driven code generation (REST handlers, clients, bindings, resource registries)
- `crates/olai-store/` — Generic resource store abstractions (Object, Association, Label, Registry)
- `crates/olai-http/` — Cloud credential abstraction and HTTP client (AWS, Azure, GCP, Databricks)

The `trestle` binary subsumes the old `proto-gen` binary; `trestle generate` and
`trestle enrich-openapi` are the canonical entry points. `trestle new` scaffolds
the four-crate "Recommended Project Structure" described in
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

Commits are GPG-signed; the message contract and signing flow are machine-wide
(`~/.claude/CLAUDE.md`). Use the `/commit` skill (`.claude/skills/commit/SKILL.md`):
commit **unsigned** as you go, then **sign and push in one step before opening a
PR** (sign → push → PR — don't push unsigned then sign, that forces a re-push).
Conventional-commit scopes are crate names; prefer small, per-crate commits.
