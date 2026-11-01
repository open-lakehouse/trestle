@AGENTS.md

## Claude Code specifics

- Commit through the `/commit` skill ([`.claude/skills/commit/SKILL.md`](.claude/skills/commit/SKILL.md)):
  lint → format → stage by name → commit, with the signing flow for GPG-signed setups.
- The `/trestle-codegen` skill covers proto annotations and what each one generates.
