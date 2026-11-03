#!/usr/bin/env bash
# Offline checks of the crates-proxy helpers: `scripts/crates-proxy/test.sh`.
#
# testdata/patched.lock is a lock resolved under a local patch whose crates also come
# from git (a crate both published and git-pinned elsewhere in the graph);
# testdata/restored.lock is what lock-registry.sh must make of it.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source-path=SCRIPTDIR source=lib.sh
source "$here/lib.sh"

fail=0
check() {
    if diff -u "$2" "$3"; then echo "ok - $1"; else echo "not ok - $1"; fail=1; fi
}

sums=" olai-store@0.0.6=$(printf '1%.0s' {1..64}) olai-store@0.0.8=$(printf '2%.0s' {1..64}) olai-uc-common@0.0.7=$(printf '3%.0s' {1..64})"
check "restores path packages, leaves git twins, qualifies their references" \
    "$here/testdata/restored.lock" \
    <(restore_registry_entries "$here/testdata/patched.lock" "$sums" "olai-http-wasm")

# config_patches reads the consumer's .cargo/config.toml.
consumer="$(mktemp -d)"
trap 'rm -rf "$consumer"' EXIT
git -C "$consumer" init -q
mkdir "$consumer/.cargo"
cat >"$consumer/.cargo/config.toml" <<'TOML'
patch.crates-io.olai-store = { path = "/up/trestle/crates/olai-store" }
patch.crates-io.olai-store-old = { path = "/up/trestle-old/crates/olai-store", package = "olai-store" }
TOML
check "config_patches reads plain and renamed patches" \
    <(printf '%s\n' "olai-store /up/trestle/crates/olai-store" "olai-store /up/trestle-old/crates/olai-store") \
    <(cd "$consumer" && config_patches)

exit "$fail"
