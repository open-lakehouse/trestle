# olai-codegen

Proto-driven code generator: turns a compiled protobuf descriptor into idiomatic
Rust server and client code, resource registries, and optional
Python/Node.js/TypeScript bindings. The single source of truth is your annotated
`.proto` files.

It's the engine behind [`olai-trestle`](https://crates.io/crates/olai-trestle)'s
`trestle generate`; this crate is the library you embed if you need codegen in
your own build tool. For how it fits the wider Trestle pipeline, see the
[architecture overview](https://github.com/open-lakehouse/trestle/blob/main/docs/architecture.md).

## What it generates

| Output | Contents |
|---|---|
| `common/` | Axum extractors, shared request/response types |
| `models/` | `labels.rs` — resource enums and `olai_store::Label` impls |
| `server/` | Handler traits (one async method per RPC) + Axum route handlers |
| `client/` | HTTP client struct with typed request builders |
| `python/` `node/` `node_ts/` | PyO3 / NAPI-RS / TypeScript bindings |

```text
proto/  ──────────────── trestle generate ────────────────▶  Rust / Python / TS
         (buf generate models · buf build descriptor · codegen)
```

## Proto annotations

The generator reads standard [Google API extensions](https://github.com/googleapis/googleapis/tree/master/google/api) — no custom annotations:

| Annotation | Effect |
|---|---|
| `google.api.resource` | Registers a managed resource type; drives resource enums and labels |
| `google.api.http` | Maps each RPC to an HTTP method/URI; drives routes and client methods |
| `google.api.field_behavior` | `IDENTIFIER`/`REQUIRED`/`OUTPUT_ONLY`/`OPTIONAL`/`INPUT_ONLY`/`IMMUTABLE`/`UNORDERED_LIST`/`NON_EMPTY_DEFAULT`; shapes extractors, builders, and store field roles |
| `google.api.resource_reference` | Parent-child relationships; drives hierarchical names |
| `debug_redact` | Marks a secret field; routed to a `SecretManager` and redacted (see `olai-store`) |

```proto
message Catalog {
  option (google.api.resource) = {
    type: "example.io/Catalog"
    pattern: "catalogs/{catalog}"
    plural: "catalogs"
    singular: "catalog"
  };
  string name = 1 [(google.api.field_behavior) = IDENTIFIER];   // maps to the store's Object.id
  string comment = 2 [(google.api.field_behavior) = OPTIONAL];
}
service CatalogService {
  rpc GetCatalog(GetCatalogRequest) returns (Catalog) {
    option (google.api.http) = { get: "/catalogs/{name}" };
  }
}
```

### Routing & hierarchy model

Unlike Google AIP's nested collections (`/catalogs/{c}/schemas/{s}/tables`),
Trestle follows the **Databricks Unity Catalog** design: every resource gets a
**flat, top-level route** (`/catalogs`, `/schemas`, `/tables`) and the parent is
named by a request field, not a URL segment. This keeps resources directly
addressable and avoids the N+1 navigation of resolving every ancestor just to
build a URL.

The logical hierarchy (`Catalog → Schema → Table`) is still encoded, but it is
**discovered** from annotations rather than the URL: each child's `List` (and
`Create`) request carries a parent-scoping field annotated
`google.api.resource_reference = { child_type: "<this service's resource>" }`.
The generator collects those edges across services and reconstructs the
depth-ordered chain, driving `parent_label`/`path_names` in the resource registry
and resource-scoped client navigation. See `docs/codegen-design.md` for the
binding-mode details.

## Usage

Codegen is driven by the `trestle` CLI. `trestle generate` runs the **whole
pipeline** — it invokes `buf generate` (model plugins), `buf build` (the proto
descriptor), then trestle's own codegen — so you only need one command:

```bash
cargo install olai-trestle              # installs the `trestle` binary
trestle generate --config trestle.yaml  # buf generate + buf build + codegen
```

This requires `buf` on your `PATH`. For environments where `buf` isn't available
(e.g. a central/Bazel build produces the descriptor upstream), pass a pre-built
`FileDescriptorSet` and the `buf` steps are skipped:

```bash
trestle generate --config trestle.yaml --descriptors path/to/api.bin
```

The config is `trestle.yaml`, a **nested** schema (proto library × servers ×
clients). See `trestle config --help` to author it, or the `TrestleConfig`
documentation in the `olai-trestle` crate for the full field reference — this
README intentionally does not duplicate the schema. `trestle new
databricks-app-rust` scaffolds a project already wired in the recommended
four-crate layout (`common` / `server` / `client` + proto) with a ready
`trestle.yaml`, so you rarely author it by hand.

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
