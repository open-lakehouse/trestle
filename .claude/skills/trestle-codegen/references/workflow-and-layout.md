# Project workflow and layout

## Scaffolding a project

`trestle new` scaffolds a project already wired in the recommended layout:

```bash
cargo install olai-trestle          # installs the `trestle` binary
trestle new databricks-app-rust     # interactive wizard
```

You rarely configure codegen by hand — the scaffold ships a working `trestle.yaml`,
`buf.yaml`, `buf.gen.yaml`, and a `justfile` with a `regen` recipe.

## The four-crate layout

```
my-app/
├── proto/<pkg>/v1/        # SOURCE OF TRUTH — you write these
│   ├── models.proto       #   resource messages + enums
│   └── service.proto      #   services + RPCs
├── buf.yaml               # proto module + deps (googleapis for google.api.*)
├── buf.gen.yaml           # serializer plugin(s) — prost or buffa runtime
├── trestle.yaml           # codegen config (paths, runtime, context/result types)
├── crates/
│   ├── common/            # shared models
│   │   └── src/models/
│   │       ├── mod.rs      #   hand-written re-exports (stable)
│   │       └── _gen/       #   GENERATED: messages + labels.rs
│   ├── server/            # Axum service
│   │   └── src/
│   │       ├── main.rs     #   hand-written Router wiring
│   │       ├── api/        #   hand-written RequestContext, Error/Result
│   │       ├── handlers/   #   hand-written handler trait impls
│   │       └── gen/        #   GENERATED: handler traits + route fns
│   └── client/            # typed HTTP client
│       └── src/gen/        #   GENERATED: client + builders
└── frontend/src/api/      # GENERATED: TypeScript client (if TS output configured)
```

Anything under `_gen/` or `gen/` is overwritten on regeneration — never edit it.

## Proto → code path

1. Write/edit `.proto` files under `proto/`.
2. Compile to a descriptor and generate:
   ```bash
   just regen          # = buf build -o api.bin && trestle generate -c trestle.yaml
   ```
   Without `just`:
   ```bash
   buf build --as-file-descriptor-set -o api.bin
   trestle generate --config trestle.yaml
   ```
3. Implement the generated handler traits and wire the `Router`.

`trestle generate --help` documents every flag; CLI flags override config values.

## `trestle.yaml` — core knobs

```yaml
descriptors: api.bin          # compiled descriptor from `buf build`
buf_gen: buf.gen.yaml         # serializer plugin config

generate:
  runtime: buffa              # protobuf runtime the GENERATED code consumes: prost (default) or buffa
  output_common: crates/common/src/models/_gen
  output_models: crates/common/src/models   # labels.rs lands in <output_models>/<models_subdir>/
  models_subdir: _gen
  output_server: crates/server/src/gen
  output_client: crates/client/src/gen
  context_type: crate::api::RequestContext  # YOUR context type (impl FromRequestParts)
  result_type:  crate::api::Result          # YOUR Result alias
  models_path_template:       my_common::models::{service}::v1   # how server/client import models
  models_path_crate_template: crate::models::{service}::v1
  generate_resource_enum:     true          # emit labels.rs (requires olai-store)
  generate_store_integration: true
  # typescript: / python: blocks add language bindings
```

The `{service}` placeholder in the path templates is substituted per service
module.

## What you write vs. what's generated

You hand-write **once, then leave stable**:

- `RequestContext` — your request-scoped context, `impl FromRequestParts`. The
  generated handler trait is generic over it (`context_type` in config).
- `Error` / `Result` types and `parse_error_response`.
- Handler trait impls — the actual business logic per RPC.
- The `Router` in `main.rs` — the generator emits one Axum handler fn per RPC but
  does **not** assemble the router, because only your app knows URL prefixes and
  which routes mix in hand-written handlers. A single struct may implement several
  handler traits; compose per-service routers with `axum::Router::merge`.
- Your store backend (`impl ObjectStore<L>`), if persisting resources.

The generator owns everything under `gen/` / `_gen/`: handler traits, route
handler fns, request extractors, typed clients + builders, `labels.rs`, and any
Python/TS/WASM bindings.

## WASM / browser clients

Generated clients are decoupled from their HTTP transport via
`transport_type_path` (default `olai_http::CloudClient`). To emit a
`wasm32`-buildable browser client, point it at `olai-http-wasm`'s `WasmClient`
(or set `transport: wasm` in `trestle.yaml`). Add an `output.wasm` directory plus
`runtime: buffa` to also emit `#[wasm_bindgen]` JS bindings and a `client.d.ts`.
See the `olai-http-wasm` README for details.
