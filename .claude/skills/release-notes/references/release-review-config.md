# `.release-review.md` — per-repo review config

This **optional** file lives at the repo root and is checked in (team-shared). It
tells the `release-notes` skill what to focus on in *this* repo: which docs to
drift-check, which code paths deserve extra scrutiny, and how the highlights
should read. It carries **review intent only** — release *mechanics* (crates,
tags, changelog grouping, publish) come from `release-plz.toml` /
`release-please-config.json`, never from here. Don't restate crate lists or tag
schemes in this file.

The skill runs fine **without** this file (it auto-discovers defaults). The
config only sharpens and prioritizes; it never *replaces* discovery, and every
field is additive to the defaults.

## Format

YAML frontmatter for the few structured fields the skill keys off, then a
markdown body of free-form review guidance the skill reads into its context
verbatim.

```markdown
---
changelog_source: release-plz        # release-plz | release-please | changelog | auto
doc_globs:                           # prose docs to drift-check (additive to defaults)
  - README.md
  - CONTRIBUTING.md
  - crates/*/README.md
critical_paths:                      # code paths whose changes warrant extra scrutiny
  - crates/olai-codegen/src/**
  - crates/olai-http/src/**/credential.rs
---

# Release review guidance

- Free-form bullets the skill treats as repo-specific review instructions.
- Mention fragile seams, cross-crate couplings, invariants to check, and the
  desired highlights voice.
```

## Frontmatter fields

| Field | Type | Default (if omitted) | Meaning |
|-------|------|----------------------|---------|
| `changelog_source` | enum | `auto` (detect `release-plz.toml`, else `release-please-config.json`, else root `CHANGELOG.md`) | Which release tool drives the changelog. Set explicitly to skip detection. |
| `doc_globs` | list of globs | `README.md`, `**/README.md`, `CONTRIBUTING.md`, `docs/**` | Prose docs to scan for drift. **Additive** to the defaults — list extras, not a replacement set. |
| `critical_paths` | list of globs | none | Code paths where any change always gets extra scrutiny in the heuristic cross-check (e.g. generated-output drift, secret redaction). |

## Guidance-body conventions

The markdown body is injected into the skill's context as repo-specific review
instructions. Useful things to capture:

- **Cross-crate couplings** — "a change in X usually implies an edit in Y".
- **Fragile seams** — areas where drift is easy and costly.
- **Invariants** — properties the cross-check should flag if violated (e.g.
  "credential types must never gain `#[derive(Debug)]`").
- **Highlights voice** — tone, audience, and what to emphasize/omit, layered on
  top of `highlights-style.md`.

Keep it short and specific. The skill already knows the generic release flow; this
body is only the repo-specific delta.
