# Golden Path: a Databricks app from proto to a dual-protocol server

> Source material for the blog post. Built and verified end-to-end against
> `examples/golden-path-app`; every claim here was run, not assumed. The
> blow-by-blow log (with exact commands and errors) is in `JOURNAL.md`.

## The story we set out to tell

You annotate a few protobuf messages and RPCs. You run one tool. You get a
Databricks-deployable Rust app whose **REST API and a buf ConnectRPC API run
side-by-side on a single port**, both backed by one hand-written business core,
with a typed client and a React frontend generated for free. That's the golden
path.

We walked it with the smallest possible example — a `Greeting` resource with
`CreateGreeting` / `GetGreeting` — to validate the *experience* and find the
gaps before committing it to a polished template.

## The arc (what actually happens)

1. **Scaffold.** `trestle new golden-path-app --app databricks-app-rust
   --runtime buffa --select app.databricks-app-rust.frontend=react` lays down a
   four-crate workspace (`common` / `server` / `client` / `frontend`), proto
   files with `google.api.*` annotations, `buf.yaml` / `buf.gen.yaml`,
   `trestle.yaml`, a `justfile`, Docker compose for the local stack, and the
   Databricks Apps deploy config.
2. **Generate.** `buf` compiles the proto descriptor and emits buffa models;
   `trestle generate` turns the annotations into REST handler traits + Axum route
   fns, a typed HTTP client, a resource registry, and a TypeScript client.
3. **Implement once.** You write a `GreetingCore` — the business logic — and a
   thin adapter that implements the generated `GreetingHandler` trait by calling
   into it.
4. **Add Connect.** One more codegen plugin (remote `buf.build/anthropics/connect-rust`) emits a
   ConnectRPC service facade **against the very same models**. A second thin
   adapter implements that trait, delegating to the same `GreetingCore`.
5. **Serve both on one port.** `main.rs` builds the REST router and mounts the
   Connect router as its fallback service on a single Axum listener.

The payoff, verified live on one port:

```
CONNECT create → REST read     (same resource, 200, identical body)
REST   create → CONNECT read   (same resource, 200, identical body)
```

Create over one protocol, read over the other — both ways. One core, two
protocols, one port.

## The design insight at the center

REST and Connect want **different-shaped handlers**:

| | REST (trestle) | Connect (`buf.build/anthropics/connect-rust`) |
|---|---|---|
| request | owned `CreateGreetingRequest` | `ServiceRequest<'_, …>` (zero-copy view) |
| context | generic `Cx = RequestContext` | `connectrpc::RequestContext` |
| result | `crate::api::Result<T>` | `ServiceResult<impl Encodable<T>>` |
| error | your `api::Error` | `connectrpc::ConnectError` |

So one struct can't satisfy both traits. But the *intent* is identical, and the
real logic is small. The clean resolution — proven here — is a **protocol-neutral
core with two thin adapters**:

```
GreetingCore  (Arc<Mutex<…>>; create/get; domain CoreError)
   ├── REST adapter     impl GreetingHandler      (CoreError → api::Error)
   └── Connect adapter  impl connect GreetingService (CoreError → ConnectError)
both mounted on ONE axum listener:
   Router::new().merge(rest).fallback_service(connect.into_axum_service())
```

And the quiet win that makes Connect cheap: the buffa models trestle already
generates for REST carry the `__buffa::view` types the Connect facade needs, so
**Connect reuses the same model crate** — no second model generation.

## What worked out of the box

- The four-crate scaffold, the proto annotations → REST handler/route/client/TS
  generation, the resource model, and the Databricks-shaped layout.
- buffa as the runtime end-to-end (models, REST, and Connect all share it).
- ConnectRPC + REST coexisting on one Axum listener (`connectrpc`'s `axum`
  feature + `into_axum_service()` made the same-port story mechanical).

## What we had to fix to get here (11 findings)

These are the gaps the run surfaced. Several were fixed in-repo during the run
(the buffa golden path was clearly under-exercised); the rest are scoped design
work. Full detail per finding is in `JOURNAL.md`.

### Fixed in-repo during the run (the REST golden path is now green)
- **F3** `trestle generate` hard-required a prost plugin even for
  `runtime: buffa` → made the models-plugin lookup runtime-aware.
- **F4** generate didn't create its output dirs → `create_dir_all` before
  canonicalize.
- **F5** the generated client crate didn't compile against the scaffold (missing
  `api`/`error`/`codegen` modules + `url`/`futures`/`olai-http` deps) → completed
  the client `lib.rs` / `Cargo.toml` templates.
- **F7** `gen` is a reserved word in Rust 2024 + the `axum` route module was
  gated on an undefinable feature → alias the module as `codegen`, scaffold the
  `axum` feature on `server`/`common`.
- **F8** the `output_common` Axum extractors were orphaned (the models `mod.rs`
  clobbered the common `mod.rs`, dropping the `pub mod <service>` that compiles
  the extractor impls) → `generate_models_mod` now re-declares them when the
  outputs are co-located.
- **Extractor bugs** the common extractors imported a wrong 1-arg `Result` alias
  and constructed buffa structs without `..Default::default()` → both fixed; the
  golden tests were re-blessed (only the two intended diffs).
- **F2** `trestle.yaml.tmpl` rendered the `typescript:` block at the wrong indent;
  **F6** the template pinned `olai-http` to an unpublished version → both fixed.

### Open design sessions (the work to make Connect first-class)
- **F11 (headline)** Connect is entirely hand-wired today. To make it part of the
  golden path, `olai-codegen` / the template should: emit the connect buf config
  (correct **absolute** `buffa_module`, per F9), scaffold the shared-core +
  two-adapter layout (or generate the Connect delegation seam), wire the
  same-port `fallback_service` in `main.rs`, and add the `connectrpc` /
  `http-body` deps (F10). The hand-wiring in `examples/golden-path-app` is the
  executable spec for this.
- **F1** app-private `--select` requires the full `app.<name>.<category>`
  namespace; the bare id is silently misrouted. DX polish.
- **Layout question** `output_common` (axum-coupled extractors) co-located with
  `output_models` (transport-neutral messages) pulls `axum` into the models
  crate. Worth deciding whether extractors belong in their own target/crate.

### Larger, deferred (named in the original plan, not yet started)
- **Local dependencies phase** — bring up the lake-based Postgres + MLflow via
  the existing trestle compose components, and decide whether/how to reconcile
  with open-lakehouse's `env-modules` compose generator. Its own design session.
- Auth/OBO context plumbing (`X-Forwarded-Access-Token`), observability wiring,
  request validation middleware, and DB migrations — smaller follow-ups.

## Bottom line for the blog

The golden path is real and the dual-protocol architecture is proven on one
port. The REST half now works from a clean `trestle new`; the Connect half works
when hand-wired and is the next chunk of generator work. The example in
`examples/golden-path-app` is the reference implementation the post is written
from, and the gaps list above is the honest roadmap.
