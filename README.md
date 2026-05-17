# Trestle

A framework for building data-platform services from protobuf definitions.

Trestle turns compiled protobuf descriptors into production-ready Rust REST APIs,
typed clients, Python (PyO3) and Node.js (NAPI) bindings, and a graph-based
resource store — all driven by proto annotations.

## Crates

| Crate | Description |
|-------|-------------|
| [`olai-codegen`](crates/olai-codegen) | Proto-driven code generation for REST handlers, clients, resource registries, and language bindings |
| [`olai-store`](crates/olai-store) | Generic, TAO-inspired object and association store with field-role enforcement |
| [`olai-http`](crates/olai-http) | HTTP client based on reqwest with built-in authorization for many clouds. |

## How it works

```text
.proto files (with google.api.resource, field_behavior, debug_redact annotations)
    │
    ▼  buf build
descriptor.bin
    │
    ▼  olai-codegen (olai-codegen)
    ├── Axum handler traits + route wiring
    ├── HTTP client structs
    ├── PyO3 bindings + .pyi typings
    ├── NAPI bindings + TypeScript client
    ├── ObjectLabel enum (impl Label)
    └── RESOURCE_DESCRIPTORS registry (field roles from annotations)
    │
    ▼  olai-store
    ├── ObjectStore<L> / AssociationStore<L>  — async CRUD + graph ops
    ├── ManagedObjectStore                    — field-role enforcement
    └── SecretManager                         — encrypted sensitive fields
```

## Quick start

Add the crates you need:

```toml
[dependencies]
olai-store = "0.1"
olai-http = "0.1"

[build-dependencies]
olai-codegen = "0.1"
```

See each crate's README for detailed usage.

## Requirements

- Rust 1.85+ (Edition 2024)
- [buf](https://buf.build/) for proto compilation (codegen only)

## License

Apache-2.0
