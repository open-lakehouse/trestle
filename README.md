# Trestle

A framework for building data-platform services from protobuf definitions.

Trestle turns compiled protobuf descriptors into production-ready Rust REST APIs,
typed clients, Python (PyO3) and Node.js (NAPI) bindings, and a graph-based
resource store — all driven by proto annotations.

## Crates

| Crate | Description |
|-------|-------------|
| [`trestle-codegen`](crates/trestle-codegen) | Proto-driven code generation for REST handlers, clients, resource registries, and language bindings |
| [`trestle-store`](crates/trestle-store) | Generic, TAO-inspired object and association store with field-role enforcement |
| [`trestle-client`](crates/trestle-client) | HTTP client based on reqwest with build in Authorization for many clouds. |

## How it works

```text
.proto files (with google.api.resource, field_behavior, debug_redact annotations)
    │
    ▼  buf build
descriptor.bin
    │
    ▼  trestle-codegen (proto-gen)
    ├── Axum handler traits + route wiring
    ├── HTTP client structs
    ├── PyO3 bindings + .pyi typings
    ├── NAPI bindings + TypeScript client
    ├── ObjectLabel enum (impl Label)
    └── RESOURCE_DESCRIPTORS registry (field roles from annotations)
    │
    ▼  trestle-store
    ├── ObjectStore<L> / AssociationStore<L>  — async CRUD + graph ops
    ├── ManagedObjectStore                    — field-role enforcement
    └── SecretManager                         — encrypted sensitive fields
```

## Quick start

Add the crates you need:

```toml
[dependencies]
trestle-store = "0.1"
trestle-cloud = "0.1"

[build-dependencies]
trestle-codegen = "0.1"
```

See each crate's README for detailed usage.

## Requirements

- Rust 1.85+ (Edition 2024)
- [buf](https://buf.build/) for proto compilation (codegen only)

## License

Apache-2.0
