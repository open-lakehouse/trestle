# Golden-Path Feasibility Run — Journal

An append-only, high-level log of the end-to-end run-through that produces the
"golden path" blog post. Each entry records what we ran, what worked, what
blocked, and the files involved. This is the raw material for the narrative.

**Target architecture:** proto-first Databricks App with a **buf ConnectRPC
server** *and* a **REST server** on the **same port**, a React frontend
consuming a generated client, minimal business logic behind a shared core.
Runtime: **buffa** throughout.

**Plan:** `~/.claude/plans/we-now-are-writing-ancient-zephyr.md`

---

## Step 0 — Record-keeping + workspace setup

- Created tracked `examples/` in the trestle repo.
- Created this `JOURNAL.md`.
- The generated example lives at `examples/golden-path-app/`.

**Scaffold invocation (resolved from `trestle new --help` + app manifest):**
- Base template defaults to `lakehouse`; the app is layered via
  `--app databricks-app-rust`.
- Frontend is an app-private category: `--select frontend=react` (default is
  already `react`); CI via `--select ci=github`.
- Runtime via `--runtime buffa`.
- `--non-interactive` skips prompts (and the `post_init` git-init hook, which is
  `confirm: true`); `-o` sets the output dir.

```
trestle new golden-path-app \
  --app databricks-app-rust \
  --runtime buffa \
  --select frontend=react \
  --non-interactive \
  -o examples/golden-path-app
```

Note: the working invocation needed the **namespaced** select
(`--select app.databricks-app-rust.frontend=react`) — see Finding 1.

## Step 1 — Scaffold + regen + build (worked, with 5 friction points)

The project scaffolds and proto→Rust/TS code generates, but the **buffa golden
path was clearly under-exercised**: the run hit five distinct gaps before the
generated client crate would even compile. Each is recorded below as a finding
(a candidate fix in the trestle repo).

### Finding 1 — App-private `--select` requires the full namespace
`--select frontend=react` fails with `component 'react' not found`. The working
form is `--select app.databricks-app-rust.frontend=react`. The bare category id
is silently misrouted to a component-name lookup.
- **DX gap:** the wizard help and `Next steps` output should show the namespaced
  form, or the resolver should accept the bare app-category id when unambiguous.
- Files: `crates/trestle/src/template/{resolve.rs,wizard.rs}`, `--select` parsing.

### Finding 2 — `trestle.yaml.tmpl` renders `typescript:` at the wrong indent
With `with_frontend`, the rendered `trestle.yaml` emits `typescript:` at column 1
(top level) instead of nested under `generate:`. `trestle generate` then errors:
`unknown field 'typescript'`. The MiniJinja `{% if with_frontend -%}` block's
2-space indent is lost.
- **Fix:** correct the indentation in
  `crates/trestle/templates/_apps/databricks-app-rust/template/trestle.yaml.tmpl`
  (the `typescript:` key must sit inside the `generate:` map).
- Worked around in the example by re-indenting `trestle.yaml`.

### Finding 3 — `trestle generate` required a prost plugin even for `runtime: buffa` (FIXED in repo)
`generate.rs::find_prost_out` unconditionally looked for a `neoeinstein-prost`
plugin in `buf.gen.yaml` to anchor the models import path — so every buffa
project failed with `no prost plugin found in buf.gen.yaml`.
- **Fix applied** (`crates/trestle/src/cli/generate.rs`): renamed to
  `find_models_out(path, runtime)`, runtime-aware — matches the `buffa` plugin
  for `Runtime::Buffa` and `prost` (excl. serde/tonic) for `Runtime::Prost`; also
  matches `local:` plugins, not just `remote:`. Parsed `runtime` before the
  lookup. This is a real generator bug on the buffa path, now fixed.

### Finding 4 — `trestle generate` does not create its output dirs
The scaffold pre-creates `crates/client/src/gen/` (with a placeholder `mod.rs`)
but **not** `crates/server/src/gen/`, and `trestle generate` does not `mkdir -p`
its output dirs, so it fails with `No such file or directory: crates/server/src/gen`.
- **Fix:** either have `trestle generate` create output dirs, or have the
  template ship a placeholder `crates/server/src/gen/mod.rs` (as it does for the
  client). Inconsistent today.
- Worked around with `mkdir -p crates/server/src/gen`.

### Finding 5 — generated client crate doesn't compile against the scaffold (BIGGEST gap)
After generation, `cargo build` fails with ~21 errors in
`golden-path-app-client`. The generated client references a supporting scaffold
the template never provides. The generated code expects the **client crate root**
to supply:
- `crate::codegen::<service>` — i.e. the `gen` module re-exported under the alias
  `codegen` (template names it `_gen` and re-exports its *contents*, not the
  module path).
- `crate::api::Result` — the configured `result_type` (an `api` module + `Result`
  alias). The golden tests use `crate::Result`; our scaffold sets
  `result_type: crate::api::Result` but provides no `api` module in the client.
- `crate::error::parse_error_response` — an `error` module with this fn.
- deps `olai-http` (`CloudClient`), `url`, `futures` — **none declared** in
  `crates/client/Cargo.toml.tmpl` (it has reqwest/serde/buffa/chrono/uuid only).

So the client `lib.rs.tmpl` + `Cargo.toml.tmpl` are incomplete: they were written
for a "models-only re-export until regen" placeholder and never updated to host
the generated client. The generated client was designed to BE the crate root
(siblings `codegen`/`api`/`error`), but the template doesn't lay those down.

- **This is a template-completeness design task**, not a one-line fix: decide the
  canonical client-crate skeleton (modules `api`/`error`/`codegen` alias, the
  `Result`/error types, the dep set) and ship it in `Cargo.toml.tmpl` +
  `lib.rs.tmpl`. Likely mirrors the server crate's `api`/`error` setup.

### Finding 6 — template pinned `olai-http` to an unpublished version
The registry (`databricks-proxy`, replacing crates-io) only has `olai-http 0.0.1`
published; the workspace's local copy is `0.0.2`. The template must pin to the
latest *published* version (`0.0.1`); release-plz keeps it in sync on publish.

### Resolution — all blocking findings fixed in the repo; golden path is green

Decision: fix the template + generator properly (not patch the example). Changes:
- **Finding 2** — `trestle.yaml.tmpl`: `{% if with_frontend %}` (no leading trim)
  so the `typescript:` block keeps its 2-space indent inside `generate:`.
- **Finding 3** — `generate.rs`: `find_prost_out` → `find_models_out(path, runtime)`,
  runtime-aware (buffa vs prost), matches `local:` + `remote:` plugins.
- **Finding 4** — `generate.rs::resolve_dir` now `create_dir_all` before
  `canonicalize`; template ships a `crates/server/src/gen/mod.rs` placeholder.
- **Finding 5** — client `lib.rs.tmpl` now provides `api` (Result + Error with
  `From` for url/serde_json/reqwest/olai_http), `error::parse_error_response`
  (decodes the `{error:{code,message}}` envelope), and the `gen` dir aliased as
  `pub mod codegen`. `Cargo.toml.tmpl` + root workspace deps gain `url`,
  `futures`, `olai-http`.
- **Finding 6** — `olai-http` pinned to `0.0.1`.
- **Finding 1** — left as a documented DX improvement (namespaced `--select`
  works; bare-id acceptance is a follow-up, not a blocker).

**Verified:** a fresh `trestle new ... --runtime buffa
--select app.databricks-app-rust.frontend=react` followed by
`buf dep update && buf build -o api.bin && buf generate && trestle generate -c
trestle.yaml && cargo build` compiles the whole workspace (server + client +
common + generated TS) with only warnings. **Step 1 complete.**

_Status: Step 1 done. Next: Step 2 — implement the GreetingHandler + wire REST
routes + run the server._

## Step 2 — wire REST end-to-end (in progress; hit 2 more generator gaps)

Implemented `GreetingHandler` for an in-memory `Service` (business core =
`Arc<Mutex<HashMap<String, Greeting>>>`) in
`crates/server/src/handlers/greeting.rs`, and wired the two routes in `main.rs`
(`POST /v1/greetings`, `GET /v1/{*name}`). Two more gaps surfaced:

### Finding 7 — `gen` is a reserved keyword in Rust 2024; `server` module gated on an undefinable feature
The scaffold targets `edition = "2024"`, where **`gen` is a reserved keyword**, so
`mod gen;` fails to parse. The generated server code itself only references
`crate::api::*` + `super::handler` (never `crate::gen`), so the fix is to mount
the output dir under an alias: `#[path = "gen/mod.rs"] mod codegen;` (matching the
client). The scaffold's `main.rs.tmpl` comment (`crate::gen::server::greeting…`)
is both keyword-invalid and path-wrong.
Also: the generated per-service `mod.rs` gates the route fns behind
`#[cfg(feature = "axum")]`, but the server crate declares no such feature *and*
`axum` is a required (non-optional) dep, so a feature literally named `axum`
can't be added the obvious way. Worked around by making `axum` an **optional**
dep with `default = ["axum"]` (optional dep auto-creates the matching feature).
- **Fixes needed:** (a) generator/template must not name the module `gen` (alias
  it, or change `output_server`/`output_common` to a non-keyword like `codegen`);
  (b) scaffold the `axum` feature in `server`/`common` `Cargo.toml.tmpl`, or make
  the generator's feature-gate name configurable.

### Finding 8 — `output_common` extractors are orphaned from the models `mod.rs` (BLOCKER)
`output_common` and `output_models` both target `crates/common/src/models/_gen`.
`output_common` writes the Axum request extractors at `_gen/greeting/server.rs`
(impls `FromRequest for CreateGreetingRequest`, `FromRequestParts for
GetGreetingRequest`) with a `_gen/greeting/mod.rs`. But the **models** generator
writes `_gen/mod.rs` declaring only `pub mod golden_path_app` — it never declares
`pub mod greeting`. So the extractor impls are **never compiled into the crate**,
and the server route fns fail with `FromRequest not implemented for
CreateGreetingRequest`.
- This is a **generator bug**: when `output_common` and `output_models` share a
  directory, the assembled `mod.rs` must declare the per-service extractor
  submodules (and gate them on the same `axum` feature). The common crate also
  needs `axum` as a (feature-gated) dependency.
- Root question for the design session: is co-locating `output_common`
  (extractors, axum-coupled) with `output_models` (pure messages) the intended
  layout? The extractors pull `axum` into the otherwise-transport-neutral models
  crate. May warrant a separate output target / crate.

### Resolution — generator fixes; REST golden path is green end-to-end

Decision: fix the generator properly. Changes:
- **Finding 7** — server `main.rs.tmpl` mounts the generated dir as
  `#[path = "gen/mod.rs"] mod codegen;` (avoids the Rust-2024 `gen` keyword) and
  now ships real wired routes (`POST /v1/greetings`, `GET /v1/{*name}`). Server &
  common `Cargo.toml.tmpl` gain an `axum` feature backed by an optional `axum`
  dep; the server's `axum` feature also enables `<common>/axum` so the extractors
  compile.
- **Finding 8** — `generate_models_mod` now takes `common_colocated` and, when the
  `output_common` extractors share the models dir (the default), re-declares the
  per-service extractor submodules (`#[cfg(feature = "axum")] pub mod <svc>;`) in
  the assembled `_gen/mod.rs` it would otherwise clobber.
- **Extractor bug A** — `server::generate_common` no longer imports the app's
  1-arg `result_type` alias (`crate::api::Result`): the extractor impls return the
  2-arg std `Result`, and the file lands in the transport-neutral models crate
  that has no `crate::api`. Removed the bogus `use`.
- **Extractor bug B** — request-struct construction in the **buffa** extractors
  now ends with `..Default::default()` (buffa messages carry a hidden
  `__buffa_unknown_fields`, so the literal can't be exhaustive). Gated to buffa
  only: prost requests are fully composed of path/query/body params, so the
  literal is already exhaustive and a spread would be a `clippy::needless_update`
  warning.
- Handler template now implements the real `GreetingHandler` (in-memory core)
  instead of a pre-regen `hello` stub; the scaffold is regen-first.

**Verified end-to-end (both the tracked example and a fresh throwaway scaffold):**
```
trestle new ... --runtime buffa  →  buf dep update/build/generate  →
trestle generate  →  cargo build  →  run server
POST /v1/greetings {"greeting":{"recipient":"world"}}
  → 200 {"name":"greetings/<uuid>","recipient":"world","message":"hello, world!"}
GET  /v1/greetings/<uuid>  → 200 (same body)
GET  /v1/greetings/nope    → 404 {"error":{"code":"NOT_FOUND","message":...}}
```
Golden tests re-blessed (only the two intended extractor diffs); all
`olai-codegen` + `olai-trestle` lib/tests/scaffold tests pass.

**Step 2 complete.** The REST half of the same-port architecture works against a
single hand-written business core (`Service` = `Arc<Mutex<HashMap>>`), which is
exactly the seam the Connect handler will also delegate into in Step 3.

_Status: Steps 1–2 done (8 findings fixed in-repo). Next: Step 3 — add the buffa
ConnectRPC server on the same port, delegating to the same `Service` core._

## Step 3 — Connect + REST on one port (THE FEASIBILITY GATE) — ✅ PASSED

Hand-wired the Connect server into the example (per the plan: prove it manually
first, then decide what to push into the generator). Result: **the target
architecture works.** One binary, one listener, one business core, serving both
buf ConnectRPC and REST.

### What was done
- **Generated the Connect facade** with `protoc-gen-connect-rust` (the installed
  local plugin) via a separate `buf.gen.connect.yaml`, output to
  `crates/server/src/connect_gen/`. **Key win: it reuses the EXISTING buffa
  models** the REST path already generates — the remote
  `buf.build/anthropics/buffa:v0.7.0` plugin emits the `__buffa::view` types at
  exactly the path the connect plugin references, so NO second model generation
  is needed. Point the connect plugin at them with
  `buffa_module=::golden_path_app_common::models` (must be an **absolute** `::`
  path — see Finding 9).
- **Extracted a shared core** (`handlers/core.rs::GreetingCore`): protocol-neutral
  `create`/`get` returning a domain `CoreError`. Both handlers delegate to it.
- **Two thin adapters over one core:** `handlers/greeting.rs` (REST
  `GreetingHandler`, maps `CoreError → api::Error`) and
  `handlers/greeting_connect.rs` (Connect `GreetingService`, maps
  `CoreError → ConnectError`, copies out of the zero-copy `ServiceRequest` with
  `.to_owned_message()`, wraps in `Response::ok`). One `Service` value backs
  both; clones share the same `Arc` core.
- **One listener:** `main.rs` builds the REST `Router`, then
  `.fallback_service(connect_router.into_axum_service())` — explicit REST paths
  win, Connect RPC paths (`/golden_path_app.v1.GreetingService/*`) fall through.

### Verified end-to-end (single port 8095, single binary)
```
CONNECT create  POST /golden_path_app.v1.GreetingService/CreateGreeting {"greeting":{"recipient":"connect-world"}}
  → 200 {"name":"greetings/<uuid>","recipient":"connect-world","message":"hello, connect-world!"}
REST    get of that resource  GET /v1/greetings/<uuid>  → 200 (SAME body)   ← cross-transport read

REST    create  POST /v1/greetings {"greeting":{"recipient":"rest-world"}}  → 200
CONNECT get     POST /golden_path_app.v1.GreetingService/GetGreeting {"name":"<that>"}  → 200 (SAME body)
```
Create on one transport, read on the other — both directions — confirms a single
shared core behind both protocols on one port.

### Findings from the spike (each → generator/template work for the real golden path)
- **Finding 9** — the connect plugin's `buffa_module`/`extern_path` target must be
  an **absolute** Rust path (`::crate::...`); a bare crate name fails its
  `check_extern_coverage` (only `::`- or `crate::`-rooted paths pass). Easy to get
  wrong; the generator that emits the buf config should always write the `::`.
- **Finding 10** — the connect plugin emits server **and** client in one file; the
  client references `::http_body::Body`, so the consuming crate needs `http-body`
  as a dep (or use the plugin's `gate_client_feature` opt to feature-gate the
  client). Server-only consumers still pay the dep unless gated.
- **Finding 11 (the big one)** — none of this is generated by trestle today. To
  make Connect a first-class part of the golden path, `olai-codegen` / the
  template need to: (a) emit the `buf.gen.connect.yaml` (or fold the connect
  plugin into the main `buf.gen.yaml`) with the correct absolute `buffa_module`;
  (b) scaffold the shared-core + two-adapter layout (or generate the Connect
  handler-trait delegation seam); (c) wire the same-port `fallback_service` in
  `main.rs.tmpl`; (d) add the `connectrpc` + `http-body` deps. The hand-wiring
  here is the spec for that work.
- **Confirmed (matches the pre-run analysis):** the REST and Connect handler
  traits are genuinely different shapes (owned req + `crate::api::Result` vs
  zero-copy `ServiceRequest` + `ServiceResult`/`ConnectError`), so a single impl
  can't satisfy both — but a shared **core** behind two thin adapters is clean and
  is the right pattern to scaffold.

**Step 3 complete — feasibility verdict: the architecture is viable and proven.**

_Status: Steps 1–3 done; the end-to-end golden path (proto → REST + Connect on one
port → shared core) runs. Next: Step 4 — synthesize the narrative + consolidated
gaps list for the blog post._
