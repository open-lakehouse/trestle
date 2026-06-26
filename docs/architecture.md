# Architecture

Trestle turns annotated protobuf into a running, typed service. This document
explains the moving parts and how a `.proto` file becomes Rust server code, typed
clients, language bindings, and a graph-backed resource store.

For a hands-on introduction, see the [top-level README](../README.md); for the
generator's annotation reference and config knobs, see
[`crates/olai-codegen/README.md`](../crates/olai-codegen/README.md).

## The pipeline

Proto definitions are the single source of truth. Everything else is derived from
them at build time:

```text
.proto files (with google.api.resource, field_behavior, debug_redact annotations)
    │
    ▼  buf build
descriptor.bin  (a compiled protobuf FileDescriptorSet)
    │
    ▼  olai-codegen  (via `trestle generate`)
    ├── Axum handler traits + route handlers   → server/
    ├── HTTP client structs (typed builders)    → client/
    ├── PyO3 bindings + .pyi typings            → python/
    ├── NAPI / TypeScript clients               → node/, node_ts/
    ├── ObjectLabel enum (impl olai_store::Label) → models/
    └── RESOURCE_DESCRIPTORS registry (field roles from annotations)
    │
    ▼  olai-store  (the runtime your service links against)
    ├── ObjectStore<L> / AssociationStore<L>  — async CRUD + graph ops
    ├── ManagedObjectStore                    — field-role enforcement
    └── SecretManager                         — encrypted sensitive fields
```

The split is deliberate: `olai-codegen` runs at **build time** (it never ships in
your binary), while `olai-store` and `olai-http` are **runtime** dependencies your
service actually links against.

## The crates

| Crate | Stage | Responsibility |
|-------|-------|----------------|
| [`olai-codegen`](../crates/olai-codegen) | build time | Reads the proto descriptor and emits Rust server/client code, language bindings, resource enums, and the field-role registry. |
| [`olai-store`](../crates/olai-store) | runtime | Generic, graph-based object/association store. Generic over `L: Label`, so any proto-defined resource taxonomy plugs in. |
| [`olai-http`](../crates/olai-http) | runtime | Cloud credential abstraction + HTTP client (AWS, Azure, GCP, Databricks) behind one `CloudClient` / `RequestSigner` seam. |
| [`olai-http-wasm`](../crates/olai-http-wasm) | runtime (browser) | The `wasm32` transport for generated clients — same client bodies, browser Fetch backend, browser-managed session. |
| [`olai-trestle`](../crates/trestle) | tooling | The `trestle` CLI: project scaffolding (`trestle new`) and the codegen entry point (`trestle generate`). |

## Key design decisions

- **Proto-driven.** All resource types, field roles, and API surface are derived
  from protobuf annotations (`google.api.resource`, `google.api.field_behavior`,
  `google.api.resource_reference`, `debug_redact`) — no custom annotation
  dialect, no hand-maintained schema alongside the proto.

- **Generic over `Label`.** The store layer is generic over `L: Label`, so any
  protobuf-defined resource taxonomy can be plugged in without changing the store.
  The concrete label enum is generated from your resource annotations.

- **Flat routing, discovered hierarchy.** Unlike Google AIP's nested collections
  (`/catalogs/{c}/schemas/{s}/tables`), Trestle follows the **Unity Catalog**
  design: every resource gets a flat, top-level route (`/catalogs`, `/schemas`,
  `/tables`) and the parent is named by a request field rather than a URL segment.
  The logical hierarchy is still encoded — it is *discovered* from
  `google.api.resource_reference` edges across services and reconstructed into a
  depth-ordered chain. See the codegen README's "Routing & hierarchy model".

- **Transport-decoupled clients.** Generated clients pick their HTTP transport via
  `transport_type_path`, so the *same* generated client body compiles against
  either the native [`olai-http`](../crates/olai-http) `CloudClient` or the
  browser [`olai-http-wasm`](../crates/olai-http-wasm) `WasmClient`.

- **No platform coupling.** The libraries are a generic framework. Library code
  avoids hardcoding any specific data platform (Unity Catalog, Hive, …); platform
  specifics live in templates and generated projects, not in the crates.

## What you write vs. what is generated

The generator draws a sharp line. Everything under a `gen/` / `_gen/` directory
is overwritten on every run — never edit it. You hand-write a small, stable
surface that the generated code refers to:

- `RequestContext` (your `impl FromRequestParts`),
- your `Error` / `Result` types and `parse_error_response`,
- the `Router` wiring (the generator emits one handler fn per RPC but does **not**
  build the router — only your app knows the URL prefixes and which routes mix in
  hand-written handlers),
- your business logic behind the generated handler traits.

`trestle new databricks-app-rust` scaffolds a project already wired in the
recommended four-crate layout (`common` / `server` / `client` + proto), so this
boundary is set up for you.
