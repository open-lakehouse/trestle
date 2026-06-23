---
name: commit
description: This skill should be used when the user asks to "commit", "prepare a commit", "stage and commit", "commit my changes", or says the work is done and changes should be committed. Commits unsigned (no interactive GPG PIN) and defers signing to a single bulk step before opening a PR.
version: 0.1.0
---

# Commit Workflow

The commit-message contract (types, `AI-assisted-by: Isaac` trailer, granularity)
lives in `~/.claude/CLAUDE.md` — follow it; this skill covers the mechanics.

Commits here are GPG-signed, and the PIN needs an interactive terminal. The agent
**commits unsigned** (`git commit --no-gpg-sign`, which needs no PIN), and the
user **signs once before pushing / opening a PR**.

trestle is a pure-Rust workspace and uses raw `cargo` (no `just`). Conventional-
commit **scopes are crate names** (`olai-store`, `olai-http`, `olai-codegen`,
`olai-http-wasm`, `olai-trestle`) — keep each commit focused on one crate where
practical.

## Workflow

### Step 1 — Lint & format
```bash
cargo clippy --all-targets --all-features -- -D warnings   # fix what it reports
cargo fmt
```
Run clippy first (it may rewrite code), then fmt.

### Step 2 — Stage specific files (and split commits)
Stage relevant files by name — never `git add -A` / `.`. When the tree spans
multiple logical changes, make **multiple small, well-scoped commits** (one
crate/type each) rather than one mixed commit — signing is one bulk step per
branch, so small commits are free and give release-plz a richer per-crate
history. Don't over-fragment.

```bash
git add <file1> <file2> ...
```

### Step 3 — Write the message, then commit unsigned
Derive a collision-safe temp path:

```bash
REPO=$(basename $(git rev-parse --show-toplevel))
BRANCH=$(git rev-parse --abbrev-ref HEAD | tr '/' '-')
MSG_FILE="/tmp/commit_msg_${REPO}_${BRANCH}.txt"
```

Write the message there with the **Write tool** (not echo/heredoc — pasted
heredocs break zsh). Format per `~/.claude/CLAUDE.md`. Then commit — no PIN:

```bash
git commit --no-gpg-sign -F "$MSG_FILE" && rm "$MSG_FILE"
```

### Step 4 — Push and open the PR (don't wait on signing)
Commit unsigned, then **push and open the PR in the same pass** — don't stop to
wait for the GPG PIN mid-flow. Tell the user the commits are unsigned and will
be signed in one bulk step at the end. Don't offer a re-sign after each commit.

## Signing — one bulk step at the end (after the PR is open)

Signing rewrites the commits (amend), so the already-pushed branch needs a
`--force-with-lease` re-push. That's safe on a solo feature branch and is
preferred over splitting work across handoffs. Surface ONE combined command for
the user to run (one GPG PIN); signatures aren't required to merge, so this can
happen any time before merge:

- One commit (HEAD):
  ```bash
  git commit --amend --no-edit -S && git push --force-with-lease
  ```
- Range (normal case):
  ```bash
  git rebase --exec 'git commit --amend --no-edit -S' "$(git merge-base main HEAD)" && git push --force-with-lease
  ```
- Verify: `git log --format='%h %G? %s' main..HEAD` — every commit shows `G`.
