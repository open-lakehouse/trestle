# Highlights style guide

The "Highlights" section is the human-written layer that sits **above** the
mechanical, auto-generated changelog in a GitHub Release. The changelog answers
*"what commits landed?"*; the highlights answer *"what does this release mean for
me, and do I need to do anything?"*. They are complementary — never replace the
changelog with highlights.

## Audience & voice

- Write for a **consumer of the crate**, not a maintainer. They care about
  behavior, API, and upgrade effort — not internal refactors.
- **Terse and concrete.** Lead with the user-visible effect ("you can now…",
  "X is faster", "Y now requires…"), not the implementation.
- No internal jargon, no PR numbers in the prose (the changelog already links
  them), no "various improvements" filler.
- Respect any `highlights voice` guidance in the repo's `.release-review.md`.

## What to include

1. **Headline changes** — the 1–3 things a reader most needs to know. A new
   capability, a notable fix, a performance win.
2. **Breaking changes & migration** — if anything breaks, this is the most
   important part: what broke, and the exact change a user makes to adapt. Always
   surface these even for a 0.x minor bump.
3. **Action required** — new required config, env vars, minimum versions, or
   one-time setup. Say "no action required" explicitly when that's true and the
   release is non-trivial.

## What to omit

- Pure-internal refactors, test-only changes, CI/chore churn (the changelog
  already hides most of these per `commit_parsers`).
- Restating every changelog line. Highlights are a **summary with judgment**, not
  a reformat.
- Anything you can't tie to a user-visible effect.

## Shape

Keep it short — a lead sentence plus a few bullets. Use an explicit
`### Breaking changes` / `### Action required` subsection only when there is
something to say.

## Worked example

Given this auto-generated `olai-codegen v0.0.2` changelog:

```markdown
### Added
- automate releases with release-plz; rename trestle crate to olai-testle (#16)
- WASM/browser client — transport seam, olai-http-wasm, and wasm-bindgen JS bindings (#14)
- add opt-in buffa runtime backend alongside prost (#13)
- better generated clients/bindings + emitter-layer cleanup (#12)
- honor name_field and fix plural handler/model naming (#11)
- scaffold project templating (#8)
### Fixed
- correctness and quality fixes for olai-codegen output (#10)
```

A good highlights section prepended above it:

```markdown
## Highlights

This release makes generated clients usable in the browser and lets you pick your
protobuf runtime.

- **Browser/WASM clients.** Generated clients can now run in the browser over a
  WASM transport (`olai-http-wasm`), with wasm-bindgen JS bindings — the same
  generated client code now targets both native and the browser.
- **Pluggable runtime.** Opt into the `buffa` runtime backend alongside the
  default `prost`, so you can match the runtime your generated project consumes.
- **Cleaner generated output.** Handler/model naming now honors `name_field` and
  pluralizes correctly; client/binding emitters were tidied up.

### Action required

No action required — these are additive. To try the browser client, pull in
`olai-http-wasm` as the transport for your generated client.
```

Note how the highlights **interpret and group** the changelog (browser story,
runtime story, output-quality story) rather than echoing each line, and call out
action explicitly even when there is none.
