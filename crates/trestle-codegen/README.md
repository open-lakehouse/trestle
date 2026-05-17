# trestle-codegen

Proto-driven code generator that transforms compiled protobuf descriptors into idiomatic Rust server and client code, resource registries, and optional Python/Node.js/TypeScript bindings.

## Overview

The single source of truth is your `.proto` files. Annotate them with standard Google API extensions and run `proto-gen generate` to emit:

| Output | What is generated |
|---|---|
| `common/` | Axum extractors, shared request/response types, `mod.rs` |
| `models/` | `labels.rs` — resource enums and `trestle_store::Label` impls |
| `server/` | Handler traits (one async method per RPC) and Axum route handlers |
| `client/` | HTTP client struct with typed request builder methods |
| `python/` | PyO3 bindings and `.pyi` type stubs |
| `node/` | NAPI-RS bindings |
| `node_ts/` | TypeScript client |

## Pipeline

```
.proto files
    │
    ▼  buf build --as-file-descriptor-set
descriptor binary (.bin)
    │
    ▼  proto-gen generate
Source files (Rust, Python, TypeScript, …)
```

## Proto Annotations

The generator reads standard Google API extensions. No custom annotations are required.

| Annotation | Effect |
|---|---|
| `google.api.resource` | Registers a message as a managed resource type; drives resource enum variants and label generation |
| `google.api.http` | Maps each RPC to an HTTP method and URI pattern; drives route handlers and client methods |
| `google.api.field_behavior` | Marks fields as `OUTPUT_ONLY`, `REQUIRED`, `OPTIONAL`, `IDENTIFIER`, etc.; shapes generated extractors and builders |
| `google.api.resource_reference` | Declares parent-child relationships between resource types; drives hierarchical name composition |

Example proto fragment (see `proto/` for full working examples):

```proto
message Catalog {
  option (google.api.resource) = {
    type: "example.io/Catalog"
    pattern: "catalogs/{catalog}"
    singular: "catalog"
    plural: "catalogs"
  };
  string name = 1 [(google.api.field_behavior) = OUTPUT_ONLY];
  string comment = 2 [(google.api.field_behavior) = OPTIONAL];
}

service CatalogService {
  rpc GetCatalog(GetCatalogRequest) returns (Catalog) {
    option (google.api.http) = { get: "/catalogs/{name}" };
  }
  rpc CreateCatalog(CreateCatalogRequest) returns (Catalog) {
    option (google.api.http) = { post: "/catalogs" body: "*" };
  }
}
```

## Usage

### Installation

```bash
cargo install --path crates/trestle-codegen --bin proto-gen
```

Or build and run directly from the workspace:

```bash
cargo run --bin proto-gen -- generate --help
```

### Compile descriptors

```bash
buf build --as-file-descriptor-set -o api.bin
```

### Config file (recommended)

A YAML config file is the preferred way to invoke `proto-gen`, particularly in multi-crate workspaces where output directories span crate boundaries. CLI flags override any value from the file.

```yaml
# proto-gen.yaml
descriptors: api.bin
buf_gen: buf.gen.yaml  # optional: auto-derives models_path_template from prost plugin output

generate:
  output_common:  crates/server/src/gen/common
  output_models:  crates/common/src/models   # labels.rs lands in <output_models>/_gen/
  output_server:  crates/server/src/gen/server
  output_client:  crates/client/src/gen/client

  context_type: my_crate::RequestContext
  result_type:  my_crate::Result

  # Required when buf_gen is not set:
  models_path_template:       my_models::models::{service}::v1
  models_path_crate_template: crate::models::{service}::v1

  # Resource enum generation (requires trestle-store)
  generate_resource_enum:      true
  generate_store_integration:  true
  generate_object_conversions: true
  error_type_path: crate::Error

  # Optional: language bindings
  python:
    output: crates/python/src/gen
    error_type: my_crate::PyError
    result_type: my_crate::PyResult

  typescript:
    output: crates/node/src/gen
    aggregate_client_name: MyClient
    client_crate_name: my_client
```

```bash
proto-gen generate --config proto-gen.yaml
```

Any field can be overridden at the command line:

```bash
proto-gen generate --config proto-gen.yaml --descriptors path/to/other.bin
```

### CLI flags (all options)

#### `proto-gen generate`

| Flag | Env var | Description |
|---|---|---|
| `--config` / `-c` | — | YAML config file; CLI flags override |
| `--descriptors` / `-d` | `UC_BUILD_DESCRIPTORS` | Compiled proto descriptor binary |
| `--context-type` | `UC_BUILD_CONTEXT_TYPE` | Rust path for the request context type |
| `--result-type` | `UC_BUILD_RESULT_TYPE` | Rust path for the `Result` alias |
| `--models-path-template` | `UC_BUILD_MODELS_PATH_TEMPLATE` | External import path template (`{service}` is replaced per service) |
| `--models-path-crate-template` | `UC_BUILD_MODELS_PATH_CRATE_TEMPLATE` | Intra-crate import path template |
| `--output-common` | `UC_BUILD_OUTPUT_COMMON` | Output dir for common extractors (required) |
| `--output-models` | `UC_BUILD_OUTPUT_MODELS` | Parent dir for resource labels |
| `--models-subdir` | `UC_BUILD_MODELS_SUBDIR` | Subdirectory inside models dir (default: `_gen`) |
| `--output-server` | `UC_BUILD_OUTPUT_SERVER` | Output dir for handler traits and route handlers |
| `--output-client` | `UC_BUILD_OUTPUT_CLIENT` | Output dir for HTTP client |
| `--output-python` | `UC_BUILD_OUTPUT_PYTHON` | Output dir for PyO3 bindings |
| `--output-node` | `UC_BUILD_OUTPUT_NODE` | Output dir for NAPI-RS bindings |
| `--output-node-ts` | `UC_BUILD_OUTPUT_NODE_TS` | Output dir for TypeScript client |
| `--python-typings-filename` | `UC_BUILD_PYTHON_TYPINGS_FILENAME` | Name of the `.pyi` stub file |

#### `proto-gen enrich-openapi`

Merges proto-derived validation rules into an OpenAPI YAML spec.

| Flag | Description |
|---|---|
| `--config` / `-c` | YAML config file (uses `enrich_openapi:` section) |
| `--spec` | OpenAPI YAML file to enrich (default: `openapi/openapi.yaml`) |
| `--jsonschema-dir` | Directory of JSON Schema files from `buf` (default: `openapi/jsonschema`) |
| `--descriptors` | Proto descriptor for deduplication pass; omit to skip |
| `--camel-case` | Convert snake_case field names to camelCase |

The same top-level config file covers both subcommands:

```yaml
descriptors: api.bin

enrich_openapi:
  spec: openapi/openapi.yaml
  jsonschema_dir: openapi/jsonschema
  camel_case: true
```

## Library Usage

`proto-gen` handles the full workflow for most use cases. If you need to embed generation into a Rust program (e.g. a custom build tool), the library API exposes the same three-step pipeline:

```rust
use trestle_codegen::{
    parse_file_descriptor_set, generate_code,
    CodeGenConfig, CodeGenOutput,
};
use protobuf::descriptor::FileDescriptorSet;
use protobuf::Message;

let descriptor_bytes = std::fs::read("api.bin")?;
let fds = FileDescriptorSet::parse_from_bytes(&descriptor_bytes)?;
let metadata = parse_file_descriptor_set(&fds)?;

let config = CodeGenConfig {
    context_type_path: "crate::RequestContext".into(),
    result_type_path: "crate::Result".into(),
    models_path_template: "my_models::models::{service}::v1".into(),
    models_path_crate_template: "crate::models::{service}::v1".into(),
    output: CodeGenOutput {
        common: "src/gen/common".into(),
        server: Some("src/gen/server".into()),
        client: Some("src/gen/client".into()),
        ..Default::default()
    },
    ..Default::default()
};

generate_code(&metadata, &config)?;
```

## Configuration Reference

### `CodeGenConfig`

| Field | Type | Description |
|---|---|---|
| `context_type_path` | `String` | Rust path for the request context type injected into handler methods |
| `result_type_path` | `String` | Rust path for the `Result` alias used in handler signatures |
| `models_path_template` | `String` | External import path for prost-generated models; `{service}` is replaced per service |
| `models_path_crate_template` | `String` | Intra-crate import path (used inside generated server/client code) |
| `output` | `CodeGenOutput` | Output directory configuration |
| `generate_resource_enum` | `bool` | Emit `Resource` and `ObjectLabel` enums in `labels.rs` |
| `generate_store_integration` | `bool` | Emit `trestle_store::Label` impl and `RESOURCE_DESCRIPTORS` static |
| `error_type_path` | `Option<String>` | Error type for `TryFrom<Resource>` impls; enables `From`/`TryFrom` generation |
| `generate_object_conversions` | `bool` | Emit `TryFrom<Object>` and `ResourceExt` impls |
| `bindings` | `Option<BindingsConfig>` | Language-binding configuration for Python/Node/TypeScript output |
| `models_gen_dir` | `Option<String>` | Relative path to prost-generated `gen/` directory |
| `resource_store_crate_name` | `String` | Name of the store crate (default: `trestle_store`) |

### `CodeGenOutput`

| Field | Type | Description |
|---|---|---|
| `common` | `PathBuf` | Required. Output dir for shared extractors and `mod.rs` |
| `models` | `Option<PathBuf>` | Parent dir for `labels.rs` and model `mod.rs` |
| `models_subdir` | `String` | Subdirectory inside `models` (default: `_gen`) |
| `server` | `Option<PathBuf>` | Output dir for handler traits and Axum route handlers |
| `client` | `Option<PathBuf>` | Output dir for HTTP client |
| `python` | `Option<PathBuf>` | Output dir for PyO3 bindings |
| `node` | `Option<PathBuf>` | Output dir for NAPI-RS bindings |
| `node_ts` | `Option<PathBuf>` | Output dir for TypeScript client |
| `python_typings_filename` | `String` | Stub filename (default: `client.pyi`) |

## Examples

The `proto/` directory contains fully-annotated example protobuf definitions (`example_models.proto`, `example_service.proto`) and a pre-compiled descriptor (`example.bin`). Integration tests in `tests/` run the full pipeline against these examples and serve as executable documentation of the generated output.
