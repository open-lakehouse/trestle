#!/usr/bin/env bash
# Print the absolute path of an upstream source checkout (trestle, mangrove, ...).
#
# Resolution order:
#   1. $<REPO>_ROOT, e.g. TRESTLE_ROOT or MANGROVE_ROOT
#   2. a directory named <repo> next to the consumer's *main* checkout, so it also
#      works from a linked worktree such as ~/code/mangrove-worktrees/<branch>
#
# Usage: upstream-root.sh <repo>
set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

repo="${1:?usage: upstream-root.sh <repo>}"
var="$(printf '%s' "$repo" | tr '[:lower:]-' '[:upper:]_')_ROOT"

if [[ -n "${!var:-}" ]]; then
    root="${!var}"
else
    common="$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null)" ||
        die "not inside a git checkout; set $var"
    main="$(dirname "$common")"
    root="$(dirname "$main")/$repo"
fi

if [[ ! -f "$root/Cargo.toml" ]]; then
    echo "error: no $repo checkout at $root" >&2
    echo "  Clone it next to the main checkout, or set $var to its path." >&2
    exit 1
fi

cd "$root" && pwd
