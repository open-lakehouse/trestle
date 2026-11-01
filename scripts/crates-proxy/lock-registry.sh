#!/usr/bin/env bash
# Turn the locally patched crates in Cargo.lock back into crates.io entries, so the
# lock can be committed while the proxy still hides the version (see
# docs/crates-proxy.md).
#
# The proxy filters only the index; tarballs come straight from the static.crates.io
# CDN. So the checksum of a published version can be computed from the tarball
# without the index, and the entry written as if Cargo had resolved it from crates.io.
#
# For each package that is patched in .cargo/config.toml (or, without a patch block,
# any non-member path package), this:
#   - fails if <name>-<version>.crate isn't on crates.io (use a git pin instead)
#   - warns if the local crate differs from its release tag, since the lock's
#     dependency list comes from the local source and must match the published one
#   - writes `source` + `checksum` and drops the matching [[patch.unused]] entries
#
# Usage: lock-registry.sh [Cargo.lock...]    # default: every committed Cargo.lock
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

patches="$(config_patches)"

for lock in "${locks[@]}"; do
    [[ -f "$lock" ]] || die "$lock not found"
    todo="$(patched_packages "$lock")"
    if [[ -n "$patches" ]]; then
        todo="$(awk 'NR == FNR {p[$1] = 1; next} p[$1]' <(echo "$patches") <(echo "$todo"))"
    fi
    # A patch the workspace doesn't use is recorded as [[patch.unused]]; drop those
    # for every patched crate, not only the restored ones.
    drop="$(printf '%s\n%s\n' "$patches" "$todo" | awk 'NF {print $1}' | tr '\n' ' ')"
    if [[ -z "$todo" ]] && ! grep -qx '\[\[patch.unused\]\]' "$lock"; then
        echo "$lock: no patched crates to restore."
        continue
    fi

    sums=""
    while read -r name ver; do
        tarball="$(mktemp)"
        if ! curl -sfL -o "$tarball" "https://static.crates.io/crates/$name/$name-$ver.crate"; then
            rm -f "$tarball"
            die "$name $ver is not published on crates.io. Commit a git pin instead:
  $name = { version = \"$ver\", git = \"<upstream repo url>\", rev = \"<sha>\" }"
        fi
        sums="$sums $name@$ver=$(shasum -a 256 "$tarball" | cut -d' ' -f1)"
        rm -f "$tarball"

        dir="$(awk -v n="$name" '$1 == n {print $2}' <<<"$patches")"
        if [[ -n "$dir" ]]; then
            upstream="$(git -C "$dir" rev-parse --show-toplevel)"
            tag=""
            for t in "$name-v$ver" "v$ver"; do
                if git -C "$upstream" rev-parse -q --verify "refs/tags/$t" >/dev/null; then
                    tag=$t
                    break
                fi
            done
            if [[ -z "$tag" ]]; then
                echo "warning: no release tag for $name $ver in $upstream (git fetch --tags?); can't check the local crate matches it" >&2
            elif ! git -C "$upstream" diff --quiet "$tag" -- "$dir"; then
                echo "warning: local $name differs from $tag; if its dependencies changed, CI --locked will reject this lock" >&2
                echo "  check out $tag in $upstream and re-run cargo, or wait for the proxy" >&2
            fi
        fi
    done < <(awk 'NF' <<<"$todo")

    awk -v sums="$sums" -v drop="$drop" -v source="$CRATES_IO_SOURCE" '
        BEGIN {
            n = split(sums, kv, " ")
            for (i = 1; i <= n; i++) { split(kv[i], a, "="); sum[a[1]] = a[2] }
            n = split(drop, d, " ")
            for (i = 1; i <= n; i++) fixed[d[i]] = 1
        }
        # Blocks are buffered so a dropped [[patch.unused]] entry goes as a whole.
        function flush() {
            if (!(kind == "unused" && fixed[name])) out = out block
            block = ""
        }
        /^\[\[/ { flush(); kind = ($0 == "[[package]]") ? "package" : ($0 == "[[patch.unused]]") ? "unused" : "other"; name = "" }
        /^name = / { name = $3; gsub(/"/, "", name) }
        /^version = / && kind == "package" {
            ver = $3; gsub(/"/, "", ver)
            block = block $0 "\n"
            if ((name "@" ver) in sum)
                block = block "source = \"" source "\"\nchecksum = \"" sum[name "@" ver] "\"\n"
            next
        }
        { block = block $0 "\n" }
        END {
            flush()
            # Dropping the last [[patch.unused]] entries leaves trailing blank lines.
            sub(/\n+$/, "\n", out)
            printf "%s", out
        }
    ' "$lock" >"$lock.tmp"
    mv "$lock.tmp" "$lock"

    echo "$lock: restored crates.io entries for:"
    awk 'NF {print "  " $1, $2}' <<<"$todo"
done

echo "Verify with check-lock.sh. Keep the patch for local builds; never commit .cargo/."
