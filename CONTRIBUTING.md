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
fire. Instead `release-binaries.yml` is invoked as an inline `workflow_call`
job (`release-binaries`) from `release-plz.yml`: the `release-plz-release` job
parses release-plz's `releases` output, and if `olai-trestle` was released this
run it passes that tag straight into the called workflow. Because the build is
gated on "the CLI was released this run", a release-plz run that didn't bump the
CLI crate is simply a no-op — no re-discovery of the freshly-created release.
`workflow_dispatch` remains for manual rebuilds against an existing tag.

- **Add a target:** extend the `matrix.include` list in
  `release-binaries.yml`. Shipping today: Linux x86_64 + aarch64 (glibc) and
  macOS aarch64 + x86_64. Intel macOS (`x86_64-apple-darwin`) is cross-compiled
  on the arm64 `macos-latest` runner (GitHub's native Intel macOS runners are
  deprecated) — this is what lets the Homebrew formula ship an Intel bottle.
  Windows, code-signing/notarization, and musl are deferred follow-ups.
- **Test without cutting a release:** run the workflow via **Run workflow**
  (`workflow_dispatch`) with an existing tag (e.g. `olai-trestle-v0.0.4`); it
  builds the matrix and uploads assets onto that existing release. Remove test
  assets afterward with `gh release delete-asset <tag> <asset>`.

### Homebrew tap (`brew install open-lakehouse/tap/trestle`)

A third job in `release-plz.yml` — `bump-homebrew` — keeps the formula in
[`open-lakehouse/homebrew-tap`](https://github.com/open-lakehouse/homebrew-tap)
in sync. It runs after `release-binaries` (same "CLI released this run" gate),
downloads the `.sha256` sidecars the binaries job just attached, regenerates
`Formula/trestle.rb` with `scripts/gen-homebrew-formula.sh`, and **opens a PR**
against the tap. A tap maintainer reviews the URL + `sha256` and merges — so a
compromised release/App credential can only propose a formula change, never
silently rewrite one (supply-chain hardening; the tap's `main` is protected and
admits no direct pushes).

- **Generator:** `scripts/gen-homebrew-formula.sh` is standalone and
  parameterized (tool/crate/version/repo/tag-prefix/checksums-dir) so it can be
  lifted into a shared org reusable workflow when a second tool needs a formula.
  It auto-detects which target archives exist and emits only those platform
  blocks. Preview a formula locally for an existing release with
  `just homebrew-formula olai-trestle-v<version>`.
- **Cross-repo credential:** the job mints a short-lived token via
  `actions/create-github-app-token` from a GitHub App installed on `homebrew-tap`
  (Contents + Pull requests: write, to push a branch and open the PR) — the
  default `GITHUB_TOKEN` cannot touch another repo, and this avoids a long-lived
  PAT. It needs two secrets in the **`release` environment**:
  `HOMEBREW_TAP_APP_ID` and `HOMEBREW_TAP_APP_PRIVATE_KEY`.
- **Verify a formula end-to-end:** `brew tap open-lakehouse/tap && brew install
  open-lakehouse/tap/trestle`, then `trestle --version`. `brew style` /
  `brew audit --strict` gate the generated file.

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

### The `@open-lakehouse/trestle-diagram` npm package

release-plz only versions Cargo packages, so the npm package `node/trestle-diagram`
is released through a placeholder crate, `trestle-diagram` (`node/Cargo.toml`).
It has no code and is never published to crates.io (`git_only` and
`publish = false` in `release-plz.toml`). Its `include` list names the sources
that end up in the npm bundle, so a commit touching them bumps the package in
the Release PR and lands in `node/trestle-diagram/CHANGELOG.md`, like a crate.
Use the `web` scope for these commits (`feat(web): …`).

When the Release PR merges, release-plz tags `trestle-diagram-v<version>` and
the `publish-trestle-diagram` job (a `workflow_call` into
`.github/workflows/publish-trestle-diagram.yml`) stamps that version into
`package.json` (whose own `version` stays `0.0.0`), then publishes the same
tarball to:

- **npmjs.com**, with trusted publishing (OIDC) and a provenance attestation.
  The trusted publisher only allows *staged* publishing, so the job runs
  `npm stage publish` and the version stays pending until a maintainer
  approves it with 2FA, on the package's npmjs.com page or with
  `npm stage approve <stage-id>` (`npm stage list` shows the id). **Every
  trestle-diagram release needs that approval.**
- **GitHub Packages**, with the job's ephemeral `GITHUB_TOKEN` (it has no OIDC
  exchange).
- **The `trestle-diagram-v<version>` GitHub Release**, as
  `open-lakehouse-trestle-diagram-<version>.tgz` plus a `.sha256`, with a
  build-provenance attestation. It covers the GitHub Packages copy too, which
  can't carry npm provenance:
  `gh attestation verify open-lakehouse-trestle-diagram-<version>.tgz --repo open-lakehouse/trestle`.

Either registry skips a version it already has, so re-running is safe. A
version that is staged but not yet approved is not visible yet, though: a
re-run tries to stage it again and fails. Approve the pending version instead. If you
add a new source file or directory to the bundle, add it to `include` in
`node/Cargo.toml` (`cargo package --list -p trestle-diagram` shows the set).

Maintainer setup (one-time). As with crates, npm can't attach a trusted publisher
to a package that doesn't exist yet, and the website can't create an empty one.
The first version is therefore *staged* with a token (`npm stage publish` can
create a package) and promoted by a maintainer, so the token can't publish on
its own. A `git_only` package has no tag until its first release, so
release-plz releases it on the first push to `main` that contains it, without a
Release PR. That run's npm job fails with a 404 until this is done:

1. Create the `open-lakehouse` org on npmjs.com. Add a granular npm token that
   can stage packages in it as `NPM_TOKEN` in the `release` environment.
2. From `main`, run **Publish trestle-diagram** for the release tag. It runs
   with `dry_run` on by default, which checks the staging command:
   ```bash
   gh workflow run publish-trestle-diagram.yml -f tag=trestle-diagram-v0.1.0 -f npm_bootstrap=true
   gh workflow run publish-trestle-diagram.yml -f tag=trestle-diagram-v0.1.0 -f npm_bootstrap=true -f dry_run=false
   ```
   GitHub Packages already has the version, and the run skips it.
3. On npmjs.com, approve the staged version (2FA). Staging a new package also
   creates a `0.0.0-stage` placeholder version.
4. Delete `NPM_TOKEN`. Then, shortly before merging the next Release PR that
   bumps `trestle-diagram`, add the trusted publisher under the package's
   **Settings → Trusted Publisher**: GitHub repo `open-lakehouse/trestle`,
   workflow `release-plz.yml` (npm checks the calling workflow), and
   environment `release`. Allow only **stage publish**; leave direct
   publishing off, so every release still needs a 2FA approval. npm expires a new trusted publisher that hasn't
   published within 2 days; if that happens, delete it and add it again.
5. In the GitHub package's settings, grant read access to repositories that
   install it from GitHub Packages.

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
