# Agent Instructions

Instructions for AI coding agents working in this repository. This is the single
canonical file: `CLAUDE.md` imports it, and Cursor/Codex read it directly. Human
contributor detail (release mechanics, publishing, auth) lives in
[`CONTRIBUTING.md`](CONTRIBUTING.md); design records live in [`docs/`](docs/).

## Project map

Multi-crate Rust workspace (edition and MSRV in the workspace `Cargo.toml`):

- `crates/trestle/` — the CLI, published as `olai-trestle` (`trestle` binary):
  `new` / `init` / `config` / `generate` / `env` / `workspace` / `enrich-openapi`,
  plus the embedded
  project templates in `crates/trestle/templates/` (excluded from the workspace)
- `crates/olai-codegen/` — proto-driven code generation (REST handlers, clients,
  bindings, resource registries)
- `crates/olai-store/` — generic resource store (Object, Association, Label,
  Registry; in-memory, SQLite and Postgres backends)
- `crates/olai-http/` — cloud credentials and HTTP client (AWS, Azure, GCP,
  Databricks) · `crates/olai-http-wasm/` — browser/WASM transport for generated
  clients
- `crates/olai-storage-credentials/`, `crates/olai-vended-store/`,
  `crates/olai-credential-vending/` — storage credential vending (model,
  client-side `object_store` integration, server-side minting; see
  [`docs/credential-vending.md`](docs/credential-vending.md))
- `crates/stack-topology/` (`olai-stack-topology`) — environment planner and
  renderer behind `trestle env` · `crates/stack-topology-wasm/` — its wasm build
  for the web UI
- `crates/olai-test-emulators/` — testcontainers storage emulators (test-only,
  unpublished)
- `node/` — Bun workspace: env editor, diagram package
  (`@open-lakehouse/trestle-diagram`), UI kit · `examples/` — standalone example
  workspaces · `test/` — env scenarios and golden fixtures

`trestle new` scaffolds the four-crate project layout described in
[`crates/olai-codegen/README.md`](crates/olai-codegen/README.md).

## Commands

| Command | Purpose |
|---|---|
| `cargo test --lib --tests` | Unit + integration tests (the CI `test` job) |
| `BLESS=1 cargo test -p olai-codegen --test golden_integration` | Re-bless the codegen golden trees |
| `just env-golden-refresh` | Re-render env scenarios into `test/expected/` |
| `just test-pg` | Postgres conformance battery (Docker) |
| `just ui-check` / `just ui-install` | Lint + typecheck / install the `node/` workspace |
| `just build-stack-wasm` | Build the wasm planner for the UI |
| `just tr <args>` | Run the trestle CLI from source |

`just --list` shows the rest. `olai-codegen` has doctests disabled
(`doctest = false`): prost-generated doc comments contain proto syntax.

## Hard rules

- **Never hand-edit generated or golden output.** Change the generator or
  template, then regenerate (`BLESS=1 …`, `just env-golden-refresh`) and commit
  the output in the same commit as its source change.
- **Never bump versions or edit changelogs by hand**; release-plz does it in the
  Release PR.
- **No platform coupling in library code.** This is a generic framework: no
  Unity Catalog, Hive, etc. in the libraries. Use generic examples in doc
  comments.
- **Credential types never derive `Debug`.** Hand-write `impl fmt::Debug` that
  renders `<redacted>` for every secret-bearing field (`Some("<redacted>")` /
  `None` for `Option`s). Reference: `AwsCredential` in
  `crates/olai-http/src/aws/credential.rs`.
- **Never commit to `main`.** Stage files by name; never `git add -A` /
  `git add .`.

## Testing

- Changes ship with tests. Never delete or comment out a passing test to make a
  suite green; fix the code, or update the test to the new correct behavior.
- Vending and signing code is tested against enforcing emulators
  (`emulator-it` feature, `olai-test-emulators`). These need Docker.
- Cover error and edge paths, not just the happy path.

## Style

Rust: `cargo fmt` + `cargo clippy`, 4-space indent. Node (`node/`): Biome.
`typos` runs in CI.

## Branches, PRs and stacks

- **PRs are squash-merged. The PR title is the commit release-plz sees**: a
  conventional commit whose scope is the crate's package name, e.g.
  `feat(olai-codegen): …` or `fix(olai-stack-topology): …`. CI lints it
  (`pr-title.yml`). Commits inside a PR are for review only.
- **One coherent change = one PR.** When work should land on `main` as separate
  commits (several crates, or change types that each deserve their own changelog
  entry or bump), use a stack with one layer per `type(scope)`.
- **Stacks** use the [`gh stack`](https://github.com/github/gh-stack) extension:
  `gh stack init`, `gh stack add`, `gh stack submit`, `gh stack sync --prune`.
  Prefer these to raw `git rebase --onto` + `gh pr edit --base`, which leaves
  stack tracking stale.
- PR body: what changed and why, a test plan, and `Closes #N` when an issue
  exists.
- **Re-trigger CI with an empty commit** (`git commit --allow-empty`), never by
  closing and reopening the PR.

### Pre-push check (mirrors CI)

```bash
cargo fmt --all --check \
  && cargo clippy --locked --all-targets -- -D warnings \
  && cargo test --locked
```

Plain `cargo test` includes doctests but respects `olai-codegen`'s
`doctest = false`. An explicit `--doc` overrides that setting and fails on the
generated proto docs.

CI also runs feature-gated clippy/test passes (`olai-store/sqlx`,
`olai-http/recording`, the vending crates' features), wasm checks and MSRV. See
`.github/workflows/ci.yml` when touching those crates.

## Builds and disk

Dev builds keep line tables for workspace crates and no debug info for
dependencies (`[profile.dev]` in `Cargo.toml`). Panic locations and backtrace
file:line for workspace code are kept. For a debugger, use
`cargo build --profile debugging` (full debug info, separate `target/debugging/`).
Run `cargo clean` in a worktree before removing it.

## Sibling repos and the crates.io proxy

mangrove, distributary, hydrofoil and headwaters consume these crates from
crates.io, and the local crates.io proxy hides releases for 7 days.
[`docs/crates-proxy.md`](docs/crates-proxy.md) explains how to build against a
fresh release anyway, using `scripts/crates-proxy/`.
