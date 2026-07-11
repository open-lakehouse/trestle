# Contributing

Thanks for contributing to Trestle! This guide covers local development and,
importantly, **how releases work** — they are fully automated by
[release-plz](https://release-plz.dev), so the commit messages you write
directly drive the published version bumps and changelogs.

## Local development

```bash
cargo build              # Build all crates
cargo test --lib --tests # Run unit + integration tests (skips doctests)
cargo test               # Run all tests including doctests
cargo clippy             # Lint
cargo fmt                # Format
```

- Rust Edition 2024, MSRV **1.88**.
- `olai-codegen` has doctests disabled (prost-generated proto doc comments
  contain proto-syntax examples that aren't valid Rust).
- Run `cargo fmt` and `cargo clippy` before opening a PR; CI enforces both.

### Commit signing

Commits must be GPG-signed. The PIN needs an interactive terminal, so the agent
commits **unsigned** (`git commit --no-gpg-sign`) and the branch is signed once
before the PR — see `~/.claude/CLAUDE.md` and the `/commit` skill for the flow.

## Conventional commits (this is the release contract)

Every commit to `main` must follow the
[Conventional Commits](https://www.conventionalcommits.org/) format, because
release-plz computes each crate's next version from the commit history:

```
<type>(<scope>): <subject>
```

- **`<type>`** drives the semver bump and the changelog section:

  | Type | Changelog section | Version effect |
  |------|--------------------|----------------|
  | `feat` | Added | minor bump |
  | `fix` | Fixed | patch bump |
  | `perf` | Performance | patch bump |
  | `refactor` | Changed | patch bump |
  | `doc` | Documentation | patch bump |
  | `build` | Build | patch bump |
  | `test`, `chore`, `ci` | _(hidden)_ | no release on its own |

- **`<scope>`** should be the affected crate, so the bump and changelog land on
  the right crate: `olai-store`, `olai-http`, `olai-codegen`, `olai-http-wasm`,
  `olai-trestle`.
- **Breaking changes** → append `!` after the type/scope (`feat(olai-store)!:`)
  or add a `BREAKING CHANGE:` footer. This triggers a **major** bump (or a minor
  bump while a crate is still `0.x`).

Examples (matching the existing history):

```
feat(olai-codegen): add opt-in buffa runtime backend
fix(olai-http): correct service-SAS string-to-sign
feat(olai-store)!: rename Label::variant to Label::kind
```

> release-plz attributes a commit to a crate by the files it touches (and the
> scope). Keep a commit focused on one crate where practical so versions and
> changelogs stay clean.

## How releases work

Releases are **independent per crate** — each crate has its own version, tag,
changelog, and crates.io publish, all managed by release-plz. There are no
manual version edits or `cargo publish` steps.

The flow:

1. **Merge normal PRs to `main`** using conventional commits (above).
2. release-plz (`.github/workflows/release-plz.yml`) opens or updates a single
   **Release PR** titled like `chore: release`. It contains, for each crate
   with releasable changes: the bumped `[package].version`, a regenerated
   `crates/<crate>/CHANGELOG.md`, and any synced inter-crate version
   requirements. Each push to `main` refreshes this PR.
3. **Review and merge the Release PR** when you're ready to cut a release.
4. On that merge, release-plz automatically:
   - creates a git tag per released crate: `<crate>-v<version>`
     (e.g. `olai-codegen-v0.1.0`),
   - creates a matching GitHub Release with the changelog as the body,
   - runs `cargo publish` for each crate **in dependency order**.

That's it — merging the Release PR is the release.

### Prebuilt CLI binaries (`cargo binstall`)

After release-plz publishes an `olai-trestle-v*` GitHub Release, a second
workflow — `.github/workflows/release-binaries.yml` — builds the `trestle` CLI
for each supported target and attaches the archives (`trestle-<target>.tar.gz`),
their `.sha256` checksums, and SLSA build-provenance attestations to **that same
release**. This lets users install a prebuilt binary instead of compiling from
source:

```bash
cargo binstall olai-trestle
# verify (optional):
gh attestation verify trestle-<target>.tar.gz --repo open-lakehouse/trestle
```

The install URL and archive layout are pinned in
`crates/trestle/[package.metadata.binstall]` and **must** stay in sync with the
workflow's `archive` / `leading-dir` settings. Only the CLI crate ships binaries;
the other crates are libraries and publish source-only to crates.io.

**How it's triggered — and why not `release: published`.** release-plz creates
the release with the default `GITHUB_TOKEN`, and GitHub does not start new
workflow runs from default-token events (fixed anti-recursion behavior; no
repo/org setting overrides it — verified). A `release:` trigger would never
fire. Instead the workflow chains off the **Release-plz workflow completing on
`main`** (`workflow_run`), then a `resolve` job finds the newest
`olai-trestle-v*` release and builds only if it doesn't already have the
binaries attached (so a release-plz run that didn't bump the CLI crate, or a
re-run, is a safe no-op).

- **Add a target:** extend the `matrix.include` list in
  `release-binaries.yml`. Linux x86_64 + aarch64 and macOS aarch64 (Apple
  Silicon) ship today. Intel macOS (`x86_64-apple-darwin`) is intentionally
  dropped — GitHub's Intel macOS runners are on a deprecation path and were
  starving the job; those users fall back to a source build. Windows,
  code-signing/notarization, and musl are deferred follow-ups.
- **Test without cutting a release:** run the workflow via **Run workflow**
  (`workflow_dispatch`) with an existing tag (e.g. `olai-trestle-v0.0.4`); it
  builds the matrix and uploads assets onto that existing release. Remove test
  assets afterward with `gh release delete-asset <tag> <asset>`.

### Per-crate versioning & the inter-crate dependency

Versions live in each crate's own `[package].version` (not in
`[workspace.package]`), so crates bump independently. The only intra-workspace
dependency is `olai-trestle → olai-codegen`. When release-plz bumps
`olai-codegen`, it automatically rewrites the `version` in `olai-trestle`'s
dependency on it, bumps `olai-trestle` accordingly, and publishes `olai-codegen`
first. You only ever maintain the `path` on that dependency; release-plz owns
the `version`.

### Published crates and the `trestle` / `olai-trestle` naming

Five crates publish to crates.io: `olai-store`, `olai-http`, `olai-codegen`,
`olai-http-wasm`, and `olai-trestle`.

The CLI crate is **published as `olai-trestle`** (the name `trestle` was already
taken on crates.io), but it still installs a binary called **`trestle`**:

```bash
cargo install olai-trestle   # installs the `trestle` command
```

All user-facing surfaces — the `trestle` binary, the `trestle.yaml` config file,
`TRESTLE_*` environment variables, and CLI help — are unchanged.

### crates.io authentication (OIDC trusted publishing)

Publishing uses crates.io **trusted publishing (OIDC)** — no long-lived
`CARGO_REGISTRY_TOKEN`. The release job has `id-token: write` and release-plz
performs the token exchange itself.

Maintainer setup (one-time, per crate):

- On crates.io, under each crate's **Settings → Trusted Publishing**, add the
  GitHub repo `open-lakehouse/trestle`, workflow `release-plz.yml`, and
  environment `release`.
- **New crate names cannot be created by OIDC** — there is no crate yet to
  attach a Trusted Publisher policy to. A brand-new publishable crate needs a
  **one-time bootstrap** first publish with a token, after which OIDC takes over.
  Run the **Bootstrap publish** workflow (`.github/workflows/bootstrap-publish.yml`,
  `workflow_dispatch`): trigger it with the package name (e.g.
  `olai-stack-topology`), leaving `dry_run` on first to confirm the package is
  publishable, then re-run with `dry_run` off to create the crate. It authenticates
  with `CARGO_REGISTRY_TOKEN` stored in the protected `release` environment. Then:
  1. On crates.io, register the Trusted Publisher for the new crate (repo
     `open-lakehouse/trestle`, workflow `release-plz.yml`, environment `release`).
  2. From then on the crate releases through `release-plz.yml` like the others;
     never run the bootstrap workflow for it again.

### GitHub authentication

The workflow uses the built-in `secrets.GITHUB_TOKEN`. The repo must have
**Settings → Actions → General → "Allow GitHub Actions to create and approve
pull requests"** enabled so release-plz can open the Release PR.

Caveat: the default token can't trigger other workflows, so **CI doesn't run on
the Release PR**. To change that, pass a fine-grained PAT / GitHub App token
(Contents + Pull-requests read/write) as `GITHUB_TOKEN` in both jobs instead.

## Configuration reference

- `release-plz.toml` — release-plz config (versioning, tags, changelog rules).
- `.github/workflows/release-plz.yml` — the two-job release workflow
  (`release-pr` and `release`).
