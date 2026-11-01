# Contributing

Thanks for contributing to Trestle. This guide covers local development and the
release process. Releases are automated by [release-plz](https://release-plz.dev).
PRs are squash-merged, so the PR title becomes the commit on `main` that
release-plz reads to compute version bumps and changelogs.

## Local development

```bash
cargo build              # Build all crates
cargo test --lib --tests # Run unit + integration tests (skips doctests)
cargo test               # Run all tests including doctests
cargo clippy             # Lint
cargo fmt                # Format
```

- Rust edition 2024, MSRV 1.88 (see `rust-version` in the workspace `Cargo.toml`).
- `olai-codegen` has doctests disabled, because prost-generated proto doc comments
  contain proto-syntax examples that are not valid Rust.
- Run `cargo fmt` and `cargo clippy` before opening a PR. CI enforces both.
- Dev builds carry no debug info for dependencies and only line tables for
  workspace crates, which keeps `target/` small. For a debugger, build with
  `--profile debugging`.
- Building a sibling repo against a trestle crate released less than a week ago?
  The crates.io proxy hides it. See [`docs/crates-proxy.md`](docs/crates-proxy.md).

### Commit signing

Sign commits with GPG (`git commit -S`, or set `commit.gpgsign = true`).

## Conventional commits (the release contract)

PR titles must follow the [Conventional Commits](https://www.conventionalcommits.org/)
format. CI lints them in `pr-title.yml`. release-plz computes each crate's next
version from these titles:

```
<type>(<scope>): <subject>
```

- `<type>` sets the semver bump and the changelog section:

  | Type | Changelog section | Version effect |
  |------|--------------------|----------------|
  | `feat` | Added | minor bump |
  | `fix` | Fixed | patch bump |
  | `perf` | Performance | patch bump |
  | `refactor` | Changed | patch bump |
  | `doc` | Documentation | patch bump |
  | `build` | Build | patch bump |
  | `test`, `chore`, `ci` | (hidden) | no release on its own |

- `<scope>` should be the affected package name, so the bump and changelog land on
  the right crate: `olai-store`, `olai-http`, `olai-codegen`, `olai-http-wasm`,
  `olai-trestle`, `olai-stack-topology`, `olai-storage-credentials`,
  `olai-vended-store`, `olai-credential-vending`, or `trestle-diagram` (the npm
  package; see below).
- A breaking change gets `!` after the type and scope (`feat(olai-store)!:`) or a
  `BREAKING CHANGE:` footer. That triggers a major bump, or a minor bump while the
  crate is still `0.x`.

Examples (matching the existing history):

```
feat(olai-codegen): add opt-in buffa runtime backend
fix(olai-http): correct service-SAS string-to-sign
feat(olai-store)!: rename Label::variant to Label::kind
```

release-plz attributes a commit to a crate by the files it touches and by the scope.
Keep a PR focused on one crate where practical, so versions and changelogs stay
clean. When a change spans several crates, or change types that each deserve their
own changelog entry, split it into a stack
([`gh stack`](https://github.com/github/gh-stack)) with one PR per `type(scope)`.
Commits inside a PR are squashed and do not reach the changelog.

## How releases work

Releases are independent per crate. Each crate has its own version, tag, changelog,
and crates.io publish, all managed by release-plz. There are no manual version edits
or `cargo publish` steps.

The flow:

1. Merge PRs to `main` with conventional-commit titles (above).
2. release-plz (`.github/workflows/release-plz.yml`) opens or updates one Release PR
   titled like `chore: release`. For each crate with releasable changes, it contains
   the bumped `[package].version`, a regenerated `crates/<crate>/CHANGELOG.md`, and
   any synced inter-crate version requirements. Each push to `main` refreshes it.
3. Review and merge the Release PR when you are ready to cut a release.
4. On that merge, release-plz:
   - creates a git tag per released crate, `<crate>-v<version>`
     (for example `olai-codegen-v0.1.0`),
   - creates a matching GitHub Release with the changelog as the body,
   - runs `cargo publish` for each crate, in dependency order.

Merging the Release PR is the release.

### Prebuilt CLI binaries (`cargo binstall`)

After release-plz publishes an `olai-trestle-v*` GitHub Release, a second
workflow, `.github/workflows/release-binaries.yml`, builds the `trestle` CLI
for each supported target and attaches the archives (`trestle-<target>.tar.gz`),
their `.sha256` checksums, and SLSA build-provenance attestations to that same
release. This lets users install a prebuilt binary instead of compiling from
source:

```bash
cargo binstall olai-trestle
# verify (optional):
gh attestation verify trestle-<target>.tar.gz --repo open-lakehouse/trestle
```

The install URL and archive layout are pinned in
`crates/trestle/[package.metadata.binstall]` and must stay in sync with the
workflow's `archive` / `leading-dir` settings. Only the CLI crate ships binaries;
the other crates are libraries and publish source-only to crates.io.

Why not a `release: published` trigger: release-plz creates the release with the
default `GITHUB_TOKEN`, and GitHub does not start new workflow runs from
default-token events. No repo or org setting overrides this, and a `release:`
trigger would never fire. Instead, `release-binaries.yml` is invoked as an inline
`workflow_call` job (`release-binaries`) from `release-plz.yml`. The
`release-plz-release` job parses release-plz's `releases` output, and if
`olai-trestle` was released in this run it passes that tag into the called
workflow. The build is gated on "the CLI was released this run", so a release-plz
run that did not bump the CLI is a no-op and does not look for the new release.
`workflow_dispatch` remains for manual rebuilds against an existing tag.

- Add a target: extend the `matrix.include` list in
  `release-binaries.yml`. Shipping today: Linux x86_64 + aarch64 (glibc) and
  macOS aarch64 + x86_64. Intel macOS (`x86_64-apple-darwin`) is cross-compiled
  on the arm64 `macos-latest` runner, because GitHub's native Intel macOS runners
  are deprecated. This is what lets the Homebrew formula ship an Intel bottle.
  Windows, code-signing/notarization, and musl are deferred follow-ups.
- Test without cutting a release: run the workflow via Run workflow
  (`workflow_dispatch`) with an existing tag (e.g. `olai-trestle-v0.0.4`); it
  builds the matrix and uploads assets onto that existing release. Remove test
  assets afterward with `gh release delete-asset <tag> <asset>`.

### Homebrew tap (`brew install open-lakehouse/tap/trestle`)

A third job in `release-plz.yml`, `bump-homebrew`, keeps the formula in
[`open-lakehouse/homebrew-tap`](https://github.com/open-lakehouse/homebrew-tap)
in sync. It runs after `release-binaries` (same "CLI released this run" gate),
downloads the `.sha256` sidecars the binaries job attached, regenerates
`Formula/trestle.rb` with `scripts/gen-homebrew-formula.sh`, and opens a PR against
the tap. A tap maintainer reviews the URL and `sha256` and merges. A compromised
release or App credential can therefore only propose a formula change, not rewrite
one. The tap's `main` is protected and admits no direct pushes.

- Generator: `scripts/gen-homebrew-formula.sh` is standalone and
  parameterized (tool/crate/version/repo/tag-prefix/checksums-dir) so it can be
  lifted into a shared org reusable workflow when a second tool needs a formula.
  It auto-detects which target archives exist and emits only those platform
  blocks. Preview a formula locally for an existing release with
  `just homebrew-formula olai-trestle-v<version>`.
- Cross-repo credential: the job mints a short-lived token via
  `actions/create-github-app-token` from a GitHub App installed on `homebrew-tap`
  (Contents + Pull requests: write, to push a branch and open the PR). The
  default `GITHUB_TOKEN` cannot touch another repo, and this avoids a long-lived
  PAT. It needs two secrets in the `release` environment:
  `HOMEBREW_TAP_APP_ID` and `HOMEBREW_TAP_APP_PRIVATE_KEY`.
- Verify a formula end-to-end: `brew tap open-lakehouse/tap && brew install
  open-lakehouse/tap/trestle`, then `trestle --version`. `brew style` /
  `brew audit --strict` gate the generated file.

### Per-crate versioning & the inter-crate dependency

Versions live in each crate's own `[package].version` (not in
`[workspace.package]`), so crates bump independently.

A dependency on another crate of this repository names both its `path` and its
`version`, normal, optional and dev-dependencies alike:

```toml
olai-http = { path = "../olai-http", version = "0.0.8" }
```

The `path` builds against the code in the repository; the `version` is what
`cargo publish` writes into the published manifest, which has no paths. When
release-plz bumps a crate, it rewrites the `version` of every dependency on it,
bumps the dependents accordingly, and publishes in dependency order. Add the
`version` once, at the dependency's current version, and leave its updates to
release-plz.

The exception is crates that are never published (`publish = false`, such as
`olai-test-emulators`): reference those by `path` only, and only as
dev-dependencies. `cargo publish` drops path-only dev-dependencies, but would
look a versioned one up on crates.io and fail.

### Published crates and the `trestle` / `olai-trestle` naming

Nine crates publish to crates.io: `olai-store`, `olai-http`, `olai-codegen`,
`olai-http-wasm`, `olai-trestle`, `olai-stack-topology`,
`olai-storage-credentials`, `olai-vended-store`, and `olai-credential-vending`.
They are the packages listed in `release-plz.toml`. `olai-test-emulators` and
`stack-topology-wasm` have `publish = false`.

The CLI crate is published as `olai-trestle`, because the name `trestle` was
already taken on crates.io. It still installs a binary called `trestle`:

```bash
cargo install olai-trestle   # installs the `trestle` command
```

The `trestle` binary, the `trestle.yaml` config file, the `TRESTLE_*` environment
variables, and the CLI help are all unchanged by the rename.

### crates.io authentication (OIDC trusted publishing)

Publishing uses crates.io trusted publishing (OIDC), with no long-lived
`CARGO_REGISTRY_TOKEN`. The release job has `id-token: write`, and release-plz
performs the token exchange itself.

Maintainer setup (one-time, per crate):

- On crates.io, under each crate's Settings → Trusted Publishing, add the GitHub
  repo `open-lakehouse/trestle`, workflow `release-plz.yml`, and environment
  `release`.
- OIDC cannot create a new crate name, because there is no crate yet to attach a
  Trusted Publisher policy to. A brand-new publishable crate needs a one-time
  bootstrap publish with a token, after which OIDC takes over. Run the
  `Bootstrap publish` workflow (`.github/workflows/bootstrap-publish.yml`,
  `workflow_dispatch`) with the package name (e.g. `olai-stack-topology`). Leave
  `dry_run` on first to confirm the package is publishable, then re-run with
  `dry_run` off to create the crate. It authenticates with `CARGO_REGISTRY_TOKEN`
  from the protected `release` environment. Then:
  1. On crates.io, register the Trusted Publisher for the new crate (repo
     `open-lakehouse/trestle`, workflow `release-plz.yml`, environment `release`).
  2. From then on the crate releases through `release-plz.yml` like the others.
     Do not run the bootstrap workflow for it again.

### The `@open-lakehouse/trestle-diagram` npm package

release-plz only versions Cargo packages, so the npm package `node/trestle-diagram`
is released through a placeholder crate, `trestle-diagram` (`node/Cargo.toml`).
It has no code and is never published to crates.io (`git_only` and
`publish = false` in `release-plz.toml`). Its `include` list names the sources
that end up in the npm bundle, so a commit touching them bumps the package in
the Release PR and lands in `node/trestle-diagram/CHANGELOG.md`, like a crate.
Use the `trestle-diagram` scope for these commits (`feat(trestle-diagram): …`).

When the Release PR merges, release-plz tags `trestle-diagram-v<version>` and
the `publish-trestle-diagram` job (a `workflow_call` into
`.github/workflows/publish-trestle-diagram.yml`) stamps that version into
`package.json` (whose own `version` stays `0.0.0`), then publishes the same
tarball to:

- npmjs.com, with trusted publishing (OIDC) and a provenance attestation. The
  trusted publisher only allows staged publishing, so the job runs
  `npm stage publish`. The version stays pending until a maintainer approves it
  with 2FA, on the package's npmjs.com page or with `npm stage approve <stage-id>`
  (`npm stage list` shows the id). Every trestle-diagram release needs that
  approval.
- GitHub Packages, with the job's ephemeral `GITHUB_TOKEN`, since it has no OIDC
  exchange.
- The `trestle-diagram-v<version>` GitHub Release, as
  `open-lakehouse-trestle-diagram-<version>.tgz` plus a `.sha256`, with a
  build-provenance attestation. The attestation also covers the GitHub Packages
  copy, which cannot carry npm provenance:
  `gh attestation verify open-lakehouse-trestle-diagram-<version>.tgz --repo open-lakehouse/trestle`.

Either registry skips a version it already has, so re-running is safe. A version
that is staged but not yet approved is not visible yet, and a re-run tries to stage
it again and fails. Approve the pending version instead. If you add a new source
file or directory to the bundle, add it to `include` in `node/Cargo.toml`
(`cargo package --list -p trestle-diagram` shows the set).

Maintainer setup (one-time). As with crates, npm cannot attach a trusted publisher
to a package that does not exist yet, and the website cannot create an empty one.
The first version is therefore staged with a token (`npm stage publish` can create
a package) and promoted by a maintainer, so the token cannot publish on its own. A
`git_only` package has no tag until its first release, so release-plz releases it
on the first push to `main` that contains it, without a Release PR. That run's npm
job fails with a 404 until this is done:

1. Create the `open-lakehouse` org on npmjs.com. Add a granular npm token that
   can stage packages in it as `NPM_TOKEN` in the `release` environment.
2. From `main`, run the Publish trestle-diagram workflow for the release tag. It runs
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
   Settings → Trusted Publisher: GitHub repo `open-lakehouse/trestle`,
   workflow `release-plz.yml` (npm checks the calling workflow), and
   environment `release`. Allow only stage publish; leave direct
   publishing off, so every release still needs a 2FA approval. npm expires a new
   trusted publisher that has not published within 2 days. If that happens, delete
   it and add it again.
5. In the GitHub package's settings, grant read access to repositories that
   install it from GitHub Packages.

### GitHub authentication

The workflow uses the built-in `secrets.GITHUB_TOKEN`. The repo must have
Settings → Actions → General → "Allow GitHub Actions to create and approve pull
requests" enabled, so release-plz can open the Release PR.

The default token cannot trigger other workflows, so CI does not run on the Release
PR. To change that, pass a fine-grained PAT or GitHub App token (Contents and
Pull requests read/write) as `GITHUB_TOKEN` in both jobs instead.

## Configuration reference

- `release-plz.toml`: release-plz config (versioning, tags, changelog rules).
- `.github/workflows/release-plz.yml`: the two-job release workflow
  (`release-pr` and `release`).
