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
    # `name = { path = "…" }` as patch.sh writes it, and the renamed form for a second
    # release of the same crate: `name-old = { path = "…", package = "name" }`.
    sed -nE \
        -e 's|^patch\.crates-io\.([A-Za-z0-9_-]+) = \{ path = "([^"]+)" \}$|\1 \2|p' \
        -e 's|^patch\.crates-io\.[A-Za-z0-9_-]+ = \{ path = "([^"]+)", package = "([A-Za-z0-9_-]+)" \}$|\2 \1|p' \
        "$config"
}

# `name dir version` of every publishable crate of a cargo workspace.
published_crates() {
    cargo metadata --no-deps --offline --format-version 1 --manifest-path "$1/Cargo.toml" 2>/dev/null |
        jq -r '.packages[] | select(.publish != []) | "\(.name) \(.manifest_path | sub("/Cargo.toml$"; "")) \(.version)"'
}

# `name dir version` of every publishable crate of every upstream patched in
# .cargo/config.toml (a crate is listed once per checkout it's patched from).
# Wider than config_patches: a patched crate pulls its upstream siblings in through
# their path dependencies, so they resolve from the checkout too without being named.
patched_upstream_crates() {
    config_patches | while read -r _ dir; do
        git -C "$dir" rev-parse --show-toplevel
    done | sort -u | while IFS= read -r root; do
        published_crates "$root"
    done
}

# Print `lock` with its restored packages turned back into crates.io entries:
#   sums  ` name@version=sha256 …`: the packages to restore, with their checksums
#   drop  `name …`: the crates whose [[patch.unused]] entries go
#
# Only a package without a `source` is restored: the same name and version can also
# be in the lock from git (a crate published from a repo that's also git-pinned),
# and that entry stays as it is. While such a twin exists, Cargo writes every
# reference to either with its source, so bare `"name version"` references, which
# meant the patched path package, get the crates.io source.
restore_registry_entries() {
    awk -v sums="$2" -v drop="$3" -v source="$CRATES_IO_SOURCE" '
        BEGIN {
            n = split(sums, kv, " ")
            for (i = 1; i <= n; i++) { split(kv[i], a, "="); sum[a[1]] = a[2] }
            n = split(drop, d, " ")
            for (i = 1; i <= n; i++) fixed[d[i]] = 1
        }
        # First pass: which restored name@version also has an entry with a source.
        FNR == NR {
            if (/^\[\[/) { pkg = ($0 == "[[package]]"); name = ver = "" }
            else if (pkg && /^name = /) { name = $3; gsub(/"/, "", name) }
            else if (pkg && /^version = /) { ver = $3; gsub(/"/, "", ver) }
            else if (pkg && /^source = / && (name "@" ver) in sum) twin[name "@" ver] = 1
            next
        }
        # Blocks are buffered so a dropped [[patch.unused]] entry goes as a whole, and
        # so a package is only restored once it is known to have no source.
        function flush() {
            if (kind == "package" && !has_source && (name "@" ver) in sum)
                sub(/\nversion = [^\n]*\n/, "&source = \"" source "\"\nchecksum = \"" sum[name "@" ver] "\"\n", block)
            if (!(kind == "unused" && fixed[name])) out = out block
            block = ""
        }
        /^\[\[/ {
            flush()
            kind = ($0 == "[[package]]") ? "package" : ($0 == "[[patch.unused]]") ? "unused" : "other"
            name = ver = ""; has_source = 0
            block = $0 "\n"
            next
        }
        /^name = / { name = $3; gsub(/"/, "", name) }
        /^version = / { ver = $3; gsub(/"/, "", ver) }
        /^source = / { has_source = 1 }
        # A dependency reference: ` "name version",`.
        /^ "[^" ]+ [^" ]+",$/ {
            ref = $1 "@" $2; gsub(/[",]/, "", ref)
            if (ref in twin) { sub(/",$/, " (" source ")\",") }
        }
        { block = block $0 "\n" }
        END {
            flush()
            # Dropping the last [[patch.unused]] entries leaves trailing blank lines.
            sub(/\n+$/, "\n", out)
            printf "%s", out
        }
    ' "$1" "$1"
}
