# olai-store

Generic, TAO-inspired resource store for typed objects and associations — the
async storage layer for services built with the
[Trestle](https://github.com/open-lakehouse/trestle) framework.

> [!WARNING]
> This store is built to get a Trestle project running quickly: it favours
> simplicity over features and performance. It's a fine default for bootstrapping,
> prototypes, and demos — but it is **not** intended for serious production
> workloads. For those, back the [traits](#store-traits) below with your own
> production-grade storage engine.

It defines the core traits and types for a graph-based store: objects (nodes),
associations (edges), field-role enforcement, and secret management. Typed
resource enums and field descriptors are usually generated from your `.proto`
files by [`olai-codegen`](https://crates.io/crates/olai-codegen).

## Add to your project

```toml
[dependencies]
olai-store = "0.0"
# `sqlx` feature adds `sqlx::FromRow` on `Object<L>`:
# olai-store = { version = "0.0", features = ["sqlx"] }
```

## Core types

- **`Label`** — a type-safe discriminant for resource types (e.g. an enum
  generated from `google.api.resource` annotations). Implement the marker trait:
  `impl olai_store::Label for MyLabel {}`.
- **`Object<L>`** — the untyped interchange format: a UUID, a label, a
  hierarchical `ResourceName`, and a JSON properties blob.
- **`Association<L>`** — a directed edge: UUID, `from_id`, label string, `to_id`,
  and optional JSON properties.
- **`ResourceName`** — a slash-separated hierarchical key, e.g.
  `"catalogs/my-catalog/schemas/my-schema"`.

## Store traits

Read and write are split so read-only callers can depend on the narrower trait:

```text
ObjectStoreReader<L>       get, get_by_name, list
ObjectStore<L>            + create, update, delete
AssociationStoreReader<L>  list
AssociationStore<L>       + add, remove
```

Implement these (`#[async_trait]`) over your backend of choice.

## Field-role enforcement

`ManagedObjectStore<L, S, M>` wraps any `ObjectStore` and enforces field roles
derived from proto annotations — built from a `RESOURCE_DESCRIPTORS` static that
`olai-codegen` generates:

| `FieldRole` | Source annotation | Behaviour |
|---|---|---|
| `Data` | (default) | Stored in properties JSON, returned as-is |
| `Identifier` | `field_behavior = IDENTIFIER` | Stripped on write; mapped to `Object.id` |
| `Managed` | `OUTPUT_ONLY` + known name | Stripped on write; injected on read (`created_at`, …) |
| `Sensitive` | `debug_redact = true` | Routed to a `SecretManager`; redacted on read |

```rust,ignore
use olai_store::{ManagedObjectStore, ResourceRegistry};

let registry = ResourceRegistry::from_static(&RESOURCE_DESCRIPTORS);
let store = ManagedObjectStore::new(backend, registry);
// With secret storage: ManagedObjectStore::with_secrets(backend, my_vault, registry)
```

`SecretManager` is a trait for encrypting `Sensitive` fields into a vault/KMS;
the default `NoSecrets` strips them instead. See the rustdoc for the full API.

## License

Apache-2.0
