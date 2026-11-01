# shellcheck shell=bash
# Shared helpers for the crates-proxy scripts. Sourced, not executed.
#
# Terms used throughout:
#   consumer  the repository the scripts act on (the current git checkout)
#   upstream  a sibling checkout whose crates the consumer depends on (trestle, mangrove, ...)

# shellcheck disable=SC2034 # used by the scripts that source this file
CRATES_IO_SOURCE='registry+https://github.com/rust-lang/crates.io-index'

die() {
    echo "error: $*" >&2
    exit 1
}

# Absolute path of the consumer checkout (the worktree, when run from one).
consumer_root() {
    git rev-parse --show-toplevel 2>/dev/null || die "not inside a git checkout"
}

# The committed Cargo.lock files of the consumer, one absolute path per line.
consumer_locks() {
    local root
    root="$(consumer_root)"
    git -C "$root" ls-files -- '*Cargo.lock' | sed "s|^|$root/|"
}

# Names of the members of every workspace in the consumer (one per committed
# Cargo.lock). A lock legitimately lists another in-repo workspace's crates as path
# dependencies, e.g. a separate wasm/ workspace depending on ../crates/*.
workspace_members() {
    local lock
    while IFS= read -r lock; do
        cargo metadata --no-deps --offline --format-version 1 \
            --manifest-path "$(dirname "$lock")/Cargo.toml" 2>/dev/null |
            jq -r '.packages[].name'
    done < <(consumer_locks)
}

# `name version source` of every [[package]] in a lock; source is `-` for a path
# dependency (a workspace member or a local patch).
lock_packages() {
    awk '
        function flush() { if (in_pkg) print name, ver, (src == "" ? "-" : src) }
        /^\[\[/ { flush(); in_pkg = ($0 == "[[package]]"); name = ver = src = ""; next }
        in_pkg && /^name = /    { name = $3; gsub(/"/, "", name) }
        in_pkg && /^version = / { ver = $3; gsub(/"/, "", ver) }
        in_pkg && /^source = /  { src = $3; gsub(/"/, "", src) }
        END { flush() }
    ' "$1"
}

# `name version` of every path package in a lock.
path_packages() {
    lock_packages "$1" | awk '$3 == "-" {print $1, $2}'
}

# `name version` of the path packages in a lock that are not workspace members:
# crates resolved through a local [patch] instead of their registry.
patched_packages() {
    local lock=$1 members
    members="$(workspace_members | tr '\n' ' ')"
    path_packages "$lock" | while read -r name ver; do
        case " $members ${CRATES_PROXY_ALLOW_PATH:-} " in
            *" $name "*) ;;
            *) echo "$name $ver" ;;
        esac
    done
}

# `name path` of every crate patched in the consumer's .cargo/config.toml by patch.sh.
config_patches() {
    local config
    config="$(consumer_root)/.cargo/config.toml"
    [[ -f "$config" ]] || return 0
    sed -nE 's|^patch\.crates-io\.([A-Za-z0-9_-]+) = \{ path = "([^"]+)" \}$|\1 \2|p' "$config"
}

# `name dir` of every publishable crate of a cargo workspace.
published_crates() {
    cargo metadata --no-deps --offline --format-version 1 --manifest-path "$1/Cargo.toml" 2>/dev/null |
        jq -r '.packages[] | select(.publish != []) | "\(.name) \(.manifest_path | sub("/Cargo.toml$"; ""))"'
}

# `name dir` of every publishable crate of every upstream patched in .cargo/config.toml.
# Wider than config_patches: a patched crate pulls its upstream siblings in through
# their path dependencies, so they resolve from the checkout too without being named.
patched_upstream_crates() {
    config_patches | while read -r _ dir; do
        git -C "$dir" rev-parse --show-toplevel
    done | sort -u | while IFS= read -r root; do
        published_crates "$root"
    done
}
