# Working behind the crates.io proxy delay

Local Cargo resolves crates.io through an age-filtering proxy
(`~/.cargo/config.toml` → `crates-proxy.cloud.databricks.com`) that hides every
version published less than 7 days ago. Only the **index** is filtered: tarballs
still come straight from the `static.crates.io` CDN, and CI resolves from
crates.io directly.

This applies to **every crate any of our repos publishes**, in whichever direction
it's consumed, not just to trestle's. The tooling below is generic: the
*upstream* is whichever sibling repo published the crate, and the *consumer* is
the repo you're building. As of this writing:

| Upstream | Publishes | Consumed by (how) |
|---|---|---|
| trestle | `olai-http`, `olai-store`, `olai-storage-credentials`, `olai-vended-store`, `olai-credential-vending`, … | mangrove, distributary, hydrofoil, headwaters, datafusion-openlineage (crates.io) |
| mangrove | `olai-uc-*` | distributary, hydrofoil (git pins) |
| breakwater | `olai-cedar-oci`, `olai-datafusion-policy*` | hydrofoil (git pin) |
| datafusion-openlineage | `datafusion-openlineage`, `openlineage-client` | hydrofoil (git pin) |
| distributary, hydrofoil | `opensharing-*`, `hydrofoil-*` | not consumed by a sibling yet |

A git pin isn't affected by the proxy, but the delay comes back the moment it's
switched to a crates.io version. So for a week after any upstream release, a
consumer that bumps to it fails locally with `failed to select a version for the
requirement` or `no matching package named`, even though CI would build it.

## Pick a strategy

1. **The version was published more than 7 days ago.** Nothing to do; the proxy
   serves it.
2. **It was published less than 7 days ago, and you need it now.** Patch the
   crates from the local upstream checkout, build and test, then restore the lock
   to crates.io entries before committing:

   ```bash
   dev/scripts/crates-proxy.sh patch <upstream>  # e.g. trestle, mangrove; gitignored .cargo/config.toml
   cargo build / cargo test ...
   dev/scripts/crates-proxy.sh lock-registry     # before every commit touching Cargo.lock
   dev/scripts/crates-proxy.sh check-lock        # the pre-commit hook runs this too
   ```

   For a faithful lock, check out the release tag (`<crate>-v<version>`, e.g.
   `olai-uc-client-v0.0.7`) in the upstream checkout. `lock-registry` warns when
   the local crate differs from it, because the lock's dependency list is taken
   from the local source. It restores every crate of a patched upstream, including
   siblings that the patched crates pull in by path.
   Drop the patch with `dev/scripts/crates-proxy.sh patch --remove <upstream>`
   once the proxy catches up.

   **Releases that chain.** If the fresh upstream release itself depends on
   another fresh release (e.g. a mangrove release that needs this week's trestle
   release), patch both: `patch mangrove` and `patch trestle`. Each upstream gets
   its own block in `.cargo/config.toml`, and `lock-registry` restores the
   crates from all of them.
3. **The change isn't released yet, or the consumer needs an unreleased fix.**
   Commit a git pin, keeping `version` so that switching back is a one-word edit:

   ```toml
   olai-uc-client = { version = "0.0.6", git = "https://github.com/open-lakehouse/mangrove", rev = "<sha>" }
   ```

   Git dependencies bypass the index, so the proxy doesn't affect them. Once the
   version is released and older than 7 days, drop `git`/`rev`
   (`grep -n 'git = ' Cargo.toml` finds the pins).

Never commit a `[patch]` section or `.cargo/config.toml`, and never commit a lock
that `check-lock` rejects.

## The scripts

They live in trestle's [`scripts/crates-proxy/`](../scripts/crates-proxy/) and act
on the git checkout they're run from (the *consumer*):

| Script | Does |
|---|---|
| `upstream-root.sh <repo>` | Prints the upstream checkout: `$<REPO>_ROOT` (e.g. `MANGROVE_ROOT`), else `<repo>` next to the consumer's **main** checkout, so it works from `~/code/<consumer>-worktrees/<branch>` too. |
| `patch.sh [--remove] <repo> [crate…]` | Writes a marked `patch.crates-io.<crate> = { path = … }` block per upstream to `.cargo/config.toml`. By default it patches every published upstream crate the consumer uses from crates.io: declared as a registry dependency in a workspace manifest (so a git pin just switched to a version is found before the lock resolves) or resolved from crates.io in a committed lock. Git-pinned crates are skipped. |
| `lock-registry.sh [lock…]` | For each locally patched crate, downloads `<name>-<version>.crate` from the CDN, writes `source` + `checksum` (the tarball's sha256, which is what crates.io records), and drops `[[patch.unused]]` entries. Fails when the version isn't published, which means use strategy 3. |
| `check-lock.sh [lock…]` | Fails when a lock has a path package that isn't a member of any workspace in the repo, or has `[[patch.unused]]` entries. Set `CRATES_PROXY_ALLOW_PATH="name …"` for intentional out-of-workspace path deps. |

Without arguments, the lock scripts cover every committed `Cargo.lock`, including
nested workspaces such as mangrove's `wasm/`.

## Consumer setup

Each consumer carries one shim, `dev/scripts/crates-proxy.sh`, that finds the
trestle checkout the same way `upstream-root.sh` does and runs the named script.
trestle is only where the tooling lives; the shim needs it even when the crates
you patch come from mangrove or breakwater. Any repo that consumes a sibling's
published crates should carry the shim. trestle itself, if it ever consumes one,
calls `scripts/crates-proxy/` directly.

```bash
#!/usr/bin/env bash
# Run a trestle crates-proxy script (patch, lock-registry, check-lock, upstream-root).
# See https://github.com/open-lakehouse/trestle/blob/main/docs/crates-proxy.md
# --if-available: exit 0 with a note when no trestle checkout is found (for hooks;
# CI's --locked build is the backstop).
set -euo pipefail
optional=0
if [[ "${1:-}" == "--if-available" ]]; then optional=1; shift; fi
cmd="${1:?usage: crates-proxy.sh [--if-available] <patch|lock-registry|check-lock|upstream-root> [args]}"
shift
if [[ -n "${TRESTLE_ROOT:-}" ]]; then
    root="$TRESTLE_ROOT"
else
    main="$(dirname "$(git rev-parse --path-format=absolute --git-common-dir)")"
    root="$(dirname "$main")/trestle"
fi
script="$root/scripts/crates-proxy/$cmd.sh"
if [[ ! -x "$script" ]]; then
    if [[ $optional -eq 1 ]]; then
        echo "note: no trestle checkout at $root; skipping crates-proxy $cmd" >&2
        exit 0
    fi
    echo "error: $script not found; clone trestle next to this repo or set TRESTLE_ROOT" >&2
    exit 1
fi
exec "$script" "$@"
```

The pre-commit hook runs `dev/scripts/crates-proxy.sh --if-available check-lock`.
`.cargo/` must be gitignored.
