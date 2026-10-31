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
    output: CodeGenOutput { common: Some("src/gen/common".into()), ..Default::default() },
    ..Default::default()
};
generate_code(&metadata, &config)?;
```

See the rustdoc for the full `CodeGenConfig` / `CodeGenOutput` field reference. A client-only
crate sets `common` to `None` to skip the Axum extractors; the `server` output needs them.

## WASM / browser clients

Generated clients are decoupled from their HTTP transport via
`transport_type_path` (default [`olai_http::CloudClient`]). Set it to
[`olai-http-wasm`](https://crates.io/crates/olai-http-wasm)'s `WasmClient` (or
`transport: wasm` in `trestle.yaml`) to emit a `wasm32`-buildable browser client;
add `output.wasm` + `runtime: buffa` to also emit `#[wasm_bindgen]` JS bindings
and a `client.d.ts`. See the `olai-http-wasm` README for details.

## REST client options

`CodeGenConfig::rest_client` (`clients.rust` in `trestle.yaml`) shapes the REST client. Every
option is off by default.

| Option | `trestle.yaml` | Effect |
|---|---|---|
| `transport_type_path = REQWEST_TRANSPORT_TYPE_PATH` | `transport: reqwest` | Emit `transport.rs`: an `HttpClient` over `reqwest::Client` with an optional bearer token. No cloud credential providers, no retries; builds for native and `wasm32`. |
| `error_module` | `error_module: { code_field, message_field, details_field, code_enum }` | Emit `error.rs`: `Error`, `Result`, `ApiError` (HTTP status, code, message, details of the JSON error body) and `parse_error_response`. The clients return this `Result`; `result_type` stays the server's. A code not in `code_enum` is kept as a string. Required with the Python bindings. |
| `json_int64_as_number` | `json_int64: number` (under `generate`) | 64-bit integers are JSON numbers, not proto3-JSON strings, and the whole numbers of `google.protobuf.Value`/`Struct`/`ListValue` fields are integers (`1`, not `1.0`). Reading accepts both. |
| `ignore_unknown_enum_values` | `ignore_unknown_enum_values: true` | An unknown enum name in a response becomes the zero value instead of failing the response. |
| `form_encoded_methods` | `form_encoded_methods: [pkg.Service.Method]` | Send the request body of these RPCs form-encoded. |
| `path_fields_in_body_methods` | `path_fields_in_body_methods: [pkg.Service.Method]` | Keep the path values in the request body of these RPCs too, for APIs that check that both match. |
| `json_query_packages` | `json_field_names_by_package: {pkg: json}` | Send the query parameters of these packages by their JSON name (`startIndex`), as their bodies. Elsewhere a query key is the proto name, or a `json_name` that differs from the default. |

Path values are percent-encoded (`/` too, except in multi-segment patterns such as
`{name=catalogs/*}`) and left out of the request body, as `google.api.http` specifies. With
`json_field_names: proto`, an explicit `json_name` is kept, and `json_field_names_by_package`
overrides the style per proto package.

## Python API

`BindingsConfig::python` (`clients.python` in `trestle.yaml`) shapes the PyO3 API. The bindings
crate and the models crate depend on `pyo3` 0.30 with the `experimental-inspect` feature, the
bindings crate also on `tokio`, and the Rust client needs the `error_module` above: the Python
exception classes raise its errors.

The generated API:

- Model classes are frozen, `eq`, with a `repr()` of `Name(field=value, …)` without the `None`
  fields. Only the models that a request takes get a keyword constructor.
- Enums are `enum.StrEnum` classes (Python 3.11+) in `<package>/_enums.py`; parameters take a
  member or a name. The zero `*_UNSPECIFIED` value is not a member; a field that holds it reads
  as `None`.
- The extension module is a declarative `#[pymodule]`, `codegen::extension_module`, with the
  proto comments as docstrings. PyO3's introspection writes the `.pyi` from the built library:
  `maturin generate-stubs` (maturin 1.16+), or the `pyo3-introspection` crate. The module
  also has `__version__`, the version of the bindings crate. Its init puts the
  submodules in `sys.modules`; for a module made with `wrap_pymodule!`, call
  `codegen::add_submodules_to_sys_modules`.
- `codegen/error.rs` holds the PyO3 exception classes: a base class (`error_base_class`;
  `bindings.error_base_class` in `trestle.yaml`) with `message`, `code`, `status` and `details`,
  and one subclass per kind of HTTP error status.
- The methods block on the Tokio runtime of `codegen/runtime.rs` (`rt()`, made on first use; it
  panics in a forked child). With `async_methods`, the module installs it in
  `pyo3-async-runtimes`.

| Option | Effect |
|---|---|
| `package` | Where `_enums.py` and the public modules go: `{ dir, name }`. Required. |
| `module` | The extension module (e.g. `my_pkg._core`). Classes go in `<module>.<service>`, the root client and the exceptions in `<module>`. Request messages get no class. Required. |
| `async_methods` | Each method gets a `<name>_async` twin that returns an `asyncio.Future` (needs `pyo3-async-runtimes`). |
| `keyword_only` | Optional parameters are keyword-only. |
| `public_modules` | A module `<package>/<service>.py` per submodule of `module`, re-exporting its classes and enums. |

In `trestle.yaml`, `package` and `module` sit under `clients.python` and the rest under
`clients.python.options`. `tests/rest_client_e2e` runs a client and Python bindings with all of
these against a server.

## Examples

The `proto/` directory has fully-annotated example definitions and a compiled
descriptor; integration tests in `tests/` run the full pipeline against them and
serve as executable documentation.

## License

Apache-2.0
