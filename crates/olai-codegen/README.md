# olai-codegen

Proto-driven code generator: turns a compiled protobuf descriptor into idiomatic
Rust server and client code, resource registries, and optional
Python/Node.js/TypeScript bindings. The single source of truth is your annotated
`.proto` files.

It's the engine behind [`olai-trestle`](https://crates.io/crates/olai-trestle)'s
`trestle generate`; this crate is the library you embed if you need codegen in
your own build tool.

## What it generates

| Output | Contents |
|---|---|
| `common/` | Axum extractors, shared request/response types |
| `models/` | `labels.rs` — resource enums and `olai_store::Label` impls |
| `server/` | Handler traits (one async method per RPC) + Axum route handlers |
| `client/` | HTTP client struct with typed request builders |
| `python/` `node/` `node_ts/` | PyO3 / NAPI-RS / TypeScript bindings |

```text
.proto  ──buf build──▶  descriptor (.bin)  ──trestle generate──▶  Rust / Python / TS
```

## Proto annotations

The generator reads standard [Google API extensions](https://github.com/googleapis/googleapis/tree/master/google/api) — no custom annotations:

| Annotation | Effect |
|---|---|
| `google.api.resource` | Registers a managed resource type; drives resource enums and labels |
| `google.api.http` | Maps each RPC to an HTTP method/URI; drives routes and client methods |
| `google.api.field_behavior` | `OUTPUT_ONLY`/`REQUIRED`/`IDENTIFIER`/…; shapes extractors and builders |
| `google.api.resource_reference` | Parent-child relationships; drives hierarchical names |

```proto
message Catalog {
  option (google.api.resource) = { type: "example.io/Catalog" pattern: "catalogs/{catalog}" };
  string name = 1 [(google.api.field_behavior) = OUTPUT_ONLY];
}
service CatalogService {
  rpc GetCatalog(GetCatalogRequest) returns (Catalog) {
    option (google.api.http) = { get: "/catalogs/{name}" };
  }
}
```

## Usage

Codegen is driven by the `trestle` CLI (a YAML config is recommended for
multi-crate workspaces). `trestle generate --help` documents every flag.

```bash
cargo install olai-trestle              # installs the `trestle` binary
buf build --as-file-descriptor-set -o api.bin
trestle generate --config trestle.yaml  # CLI flags override config values
```

```yaml
# trestle.yaml (core knobs; see `trestle generate --help` for the rest)
descriptors: api.bin
generate:
  output_common: crates/server/src/gen/common
  output_models: crates/common/src/models     # labels.rs lands in <output_models>/_gen/
  output_server: crates/server/src/gen/server
  output_client: crates/client/src/gen/client
  context_type:  my_crate::RequestContext
  result_type:   my_crate::Result
  models_path_template:       my_models::models::{service}::v1
  models_path_crate_template: crate::models::{service}::v1
  generate_resource_enum:     true             # requires olai-store
  generate_store_integration: true
  # python: / typescript: blocks add language bindings
```

`trestle new databricks-app-rust` scaffolds a project already wired in the
recommended four-crate layout (`common` / `server` / `client` + proto), so you
rarely set this up by hand.

## What you write vs. generate

You hand-write (once, then stable): `RequestContext` (impl
`FromRequestParts`), your `Error`/`Result` types, and `parse_error_response`.
Everything under a `gen/`/`_gen/` directory is overwritten each run — don't edit
it. The generator emits one Axum handler fn per RPC but does **not** build the
`Router`: you wire routes by hand, since only your app knows the URL prefixes and
which routes mix in hand-written handlers.

## Library API

If you embed generation directly, the pipeline is parse → configure → generate:

```rust,ignore
use olai_codegen::{parse_file_descriptor_set, generate_code, CodeGenConfig, CodeGenOutput};

let fds = /* protobuf::descriptor::FileDescriptorSet parsed from api.bin */;
let metadata = parse_file_descriptor_set(&fds)?;
let config = CodeGenConfig {
    context_type_path: "crate::RequestContext".into(),
    result_type_path: "crate::Result".into(),
    output: CodeGenOutput { common: "src/gen/common".into(), ..Default::default() },
    ..Default::default()
};
generate_code(&metadata, &config)?;
```

See the rustdoc for the full `CodeGenConfig` / `CodeGenOutput` field reference.

## WASM / browser clients

Generated clients are decoupled from their HTTP transport via
`transport_type_path` (default [`olai_http::CloudClient`]). Set it to
[`olai-http-wasm`](https://crates.io/crates/olai-http-wasm)'s `WasmClient` (or
`transport: wasm` in `trestle.yaml`) to emit a `wasm32`-buildable browser client;
add `output.wasm` + `runtime: buffa` to also emit `#[wasm_bindgen]` JS bindings
and a `client.d.ts`. See the `olai-http-wasm` README for details.

## Examples

The `proto/` directory has fully-annotated example definitions and a compiled
descriptor; integration tests in `tests/` run the full pipeline against them and
serve as executable documentation.

## License

Apache-2.0
