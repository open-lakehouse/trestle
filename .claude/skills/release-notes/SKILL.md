---
name: release-notes
description: This skill should be used when preparing or polishing a release driven by release-plz (or release-please) — when the user wants to "prep the release", "review the release PR", "augment the release notes", "add highlights to the GitHub release", or otherwise turn the mechanical auto-generated changelog into a cleanly-reviewed, high-quality release. Two modes — pre-release prep (analyze the open Release PR, cross-check commits, find doc drift, open a blocking doc-fix PR, draft highlights) and post-release augmentation (prepend curated highlights to the published GitHub Releases).
version: 0.1.0
---

# Release Notes Workflow

release-plz already produces mechanically-correct per-crate releases: it computes
semver bumps from conventional commits, regenerates per-crate `CHANGELOG.md`
files, and on Release-PR merge creates per-crate git tags + GitHub Releases with
the changelog as the body. That output is accurate but **flat** — a grouped list
of `feat`/`fix`/`doc` lines with no narrative about *what actually matters* to a
user, and no guarantee the repo's prose docs still match the shipping code.

This skill closes that gap. It has **two modes**, chosen from the skill args:

- **`prep`** — *before* merging the Release PR. Analyze what will ship, sanity-
  check commit metadata, find drifted docs, open a blocking doc-fix PR, and draft
  the human "highlights" text.
- **`post-release`** — *after* the Release PR merges and the GitHub Releases
  exist. Prepend the curated highlights above each Release's mechanical changelog.

Default to **`prep`** when the mode is ambiguous, and state which mode is running.

This skill is **repo-agnostic**. It reads release *mechanics* from the repo's
release-tool config (`release-plz.toml` / `release-please-config.json`) and
release *review intent* from an optional `.release-review.md` at the repo root.
Never hardcode crate names, tag schemes, or doc paths — resolve them at run time.

---

## Mode A — `prep` (before merging the Release PR)

### Step 0 — Load configs

**(a) Release-tool config (source of truth for mechanics).** Detect and parse,
in order:

1. `release-plz.toml` — read:
   - `[[package]]` blocks → the releasable crates and their `changelog_path`s.
   - `git_tag_name` / `git_release_name` / `git_release_body` → the tag and
     release-title scheme (used in Mode B to locate releases).
   - `[changelog].commit_parsers` → this repo's type→section map and which types
     are hidden (so the cross-check reasons about *this repo's* releasable-vs-
     hidden rules, not assumed defaults).
   - `semver_check` → whether release-plz already guards breaking changes.
   - `git_release_enable` / `publish` → whether Releases / crates.io publishes
     actually happen.
2. else `release-please-config.json` (read its package + changelog config);
3. else fall back to bare `CHANGELOG.md` heuristics and note the reduced fidelity.

**(b) Review config (optional, additive).** Read `.release-review.md` at the repo
root if present: parse the YAML frontmatter for `changelog_source`, `doc_globs`,
`critical_paths`; inject the markdown guidance body verbatim into your working
context as repo-specific review instructions. If absent, auto-discover defaults
(see `references/release-review-config.md`) and proceed — the config only
sharpens focus, it is never required. The schema is documented in
`references/release-review-config.md`.

### Step 1 — Locate the Release PR

```bash
gh pr list --label release --state open --json number,title,headRefName,url
```

If none is open, tell the user release-plz hasn't opened a Release PR yet (a push
to `main` is needed to refresh it) and **stop** — do not fabricate the release
shape from local state.

### Step 2 — Extract what ships

```bash
gh pr view <n> --json files,body
gh pr diff <n>
```

The signal is the diff of each crate's `CHANGELOG.md` (the new `## [x.y.z]`
sections) and each bumped `[package].version`. Build a per-crate table:

| Crate | Old → New | Bump | New changelog lines |
|-------|-----------|------|---------------------|

Derive bump kind (major / minor / patch) from the version delta; note that a 0.x
crate gets a *minor* bump for breaking changes.

### Step 3 — Heuristic commit cross-check (inline, cheap)

For each new changelog entry, map it back to its commit (`git log`, match by
subject) and the files that commit touched. Flag **obvious** mismatches only:

- a `fix`/`feat` whose diff changes a public signature but isn't marked breaking
  (possible missing `!`) — weight this higher when `semver_check` is *off*, since
  release-plz then has no safety net;
- a `feat` that touches no public API (maybe should be `fix`/`refactor`);
- a changelog line landing on the wrong crate vs. the files touched.

Report flags as **suggestions**. Never rewrite git history automatically.

**Escalation to deep review (opt-in).** If the heuristic pass flags anything, ask
the user whether to run a deep audit of *only* the flagged commits. On accept,
spawn **one subagent per flagged commit** (`Agent`, `general-purpose`), each
reading that commit's full diff and verifying type/scope correctness and
breaking-change classification against the actual public API surface. Have each
return a structured verdict — `correct` / `wrong-type` / `missing-breaking` /
`wrong-crate` — with reasoning. Present the verdicts as recommended commit-
message / PR fixes for the user to apply before merging the Release PR. Still
never rewrite history automatically. Keep this scoped to the flagged set so the
default prep run stays cheap.

### Step 4 — Doc drift scan

Build the doc set from `.release-review.md`'s `doc_globs` (or the auto-discovered
defaults), filtered to docs plausibly related to the crates that bump. Fold in
the config's `critical_paths` and prose guidance as extra scrutiny. Read each doc
and check the prose against the shipping change:

- renamed / added / removed public API or types;
- changed CLI flags, config-file keys, or environment variables;
- new or removed generated outputs / templates;
- changed install or usage instructions.

Produce a concrete list of doc edits needed **before** release.

### Step 5 — Open the blocking doc-fix PR (only if real drift was found)

- Branch off `main`, apply the doc edits, and commit via the **`commit` skill**
  (commit unsigned → push → open PR → surface the one bulk sign command). Do
  **not** re-implement the GPG signing flow here. Use `doc(<scope>):` commits so
  they themselves flow through release-plz.
- Cross-link the PRs so the gate is visible:
  ```bash
  gh pr comment <release-pr> --body "Blocked by #<doc-pr> (pre-release doc fixes)."
  gh pr edit <release-pr> --body "<existing body>\n\n- [ ] #<doc-pr> merged"
  ```
- Be honest in your report: "blocking" here is a **review convention**, not a
  hard GitHub merge-gate, unless branch protection / required-status checks are
  configured.
- Note that merging the doc PR re-triggers release-plz, which folds the new
  `doc:` entries into the Release PR automatically — so the changelog (and your
  Step 2 table) will refresh.

### Step 6 — Draft the highlights

For each releasing crate, draft a "Highlights / What this means for you" section
following `references/highlights-style.md` and any `highlights voice` guidance in
`.release-review.md`. Save the draft to the session scratchpad as a reviewable
artifact and surface its path — it is applied in Mode B.

**Mode A output:** the per-crate change table + bump kinds, any commit-metadata
flags (and deep-audit verdicts if run), the doc PR link (if opened), and the
saved highlights draft path.

---

## Mode B — `post-release` (after the Release PR is merged)

### Step 1 — Find the just-published releases

```bash
gh release list --json tagName,name,createdAt
```

(`url` isn't a valid field on `gh release list`; get it from
`gh release view <tag> --json url` when reporting in Step 3.)

Match against the tag scheme resolved from `git_tag_name` in Step 0 (do **not**
assume `<crate>-v<version>` — read it). Default to the set created by the most
recent Release-PR merge; confirm with the user if the target set is ambiguous.

### Step 2 — Augment each Release body

For each target release:

1. Read the current body (`gh release view <tag> --json body`).
2. Prepend a `## Highlights` / "What this means for you" section — from the Mode A
   draft, or regenerated from the release's own changelog if no draft exists —
   **above** the existing release-plz changelog.
3. Apply it:
   ```bash
   gh release edit <tag> --notes "<combined body>"
   ```

**Never discard the mechanical changelog — augment, not replace.** Keep highlights
in the GitHub Release only (never in `CHANGELOG.md`), so release-plz's changelog
regeneration can't clobber the curated prose.

### Step 3 — Report

List which releases were edited, with their URLs.

---

## Workflow notes & gotchas

- **CI doesn't run on the Release PR** (default `GITHUB_TOKEN` can't trigger other
  workflows — see `CONTRIBUTING.md`). The doc-fix PR is a normal PR and *does* get
  CI, so landing doc fixes there rather than editing the Release PR directly keeps
  the gated path. Prefer it.
- **Run `prep` early and re-run it** as more PRs land — the Release PR is
  continuously refreshed by release-plz, so the change table is a live preview,
  not a one-shot.
- **Don't restate the commit contract or release flow** — they live in
  `CONTRIBUTING.md`; reference it.
