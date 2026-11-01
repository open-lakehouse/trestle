#!/usr/bin/env bash
# Fail if a Cargo.lock still resolves a crate from a local [patch] (a path package
# that isn't a member of any workspace in the repo) or lists [[patch.unused]]
# entries. CI resolves from crates.io with --locked, so such a lock fails there.
# Meant for pre-commit.
#
# Set CRATES_PROXY_ALLOW_PATH="name ..." for intentional out-of-workspace path deps.
#
# Usage: check-lock.sh [Cargo.lock...]    # default: every committed Cargo.lock
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
source "$here/lib.sh"

if [[ $# -gt 0 ]]; then
    locks=("$@")
else
    locks=()
    while IFS= read -r lock; do locks+=("$lock"); done < <(consumer_locks)
fi

status=0
for lock in "${locks[@]}"; do
    [[ -f "$lock" ]] || die "$lock not found"
    bad="$(patched_packages "$lock")"
    if grep -qx '\[\[patch.unused\]\]' "$lock"; then
        bad="$bad${bad:+$'\n'}[[patch.unused]] entries"
    fi
    if [[ -n "$bad" ]]; then
        echo "error: $lock was resolved under a local [patch]:" >&2
        awk '{print "  " $0}' <<<"$bad" >&2
        status=1
    fi
done

if [[ $status -ne 0 ]]; then
    echo "Restore the crates.io entries with $here/lock-registry.sh" >&2
    echo "(or, outside the proxy delay window, git checkout main -- <lock> and rebuild)." >&2
    exit 1
fi
echo "Cargo.lock files resolve every non-member crate from a registry or git."
