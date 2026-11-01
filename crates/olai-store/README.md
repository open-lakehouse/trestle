# olai-store

Generic, TAO-inspired resource store for typed objects and associations — the
async storage layer for services built with the
[Trestle](https://github.com/open-lakehouse/trestle) framework.

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
| `Sensitive` | `debug_redact = true` | Sealed into an encrypted blob stored inline on the object row; redacted on read |

```rust,ignore
use olai_store::{ManagedObjectStore, ResourceRegistry};

let registry = ResourceRegistry::from_static(&RESOURCE_DESCRIPTORS);
let store = ManagedObjectStore::new(backend, registry);
// With envelope encryption (needs the `encryption` feature):
// ManagedObjectStore::with_encryptor(backend, my_encryptor, registry)
```

`ManagedObjectStore` is the primary store API; the raw `InMemoryStore` / `SqlStore`
backends are a taxonomy-blind blob layer it wraps. With the `encryption` feature,
`Sensitive` fields are sealed by an injectable `EnvelopeEncryptor` (AES-256-GCM
envelope encryption; pluggable `KeyProvider` for a local KEK or a cloud KMS) and
stored — atomically with the object — in a nullable `sensitive` blob column.
See the rustdoc for the full API.

## Indexed property paths

`search` pushes filters into SQL, but by default no index can serve them. To make
string lookups on a hot payload field indexable, declare it per label on
`SqlStore` / `PgStore`, then create the indexes as an explicit step, like
migrations:

```rust,ignore
use olai_store::PropertyIndex;

let store = PgStore::connect(pool)
    .with_property_indexes([PropertyIndex::new(Kind::Table, "location_key")?]);
store.ensure_property_indexes().await?; // idempotent CREATE INDEX IF NOT EXISTS
```

Exact matches, `Or` lists of `eq`, and ranges (`ge` + `lt`, e.g. a bytewise
prefix) on a declared path then use a `(label, <path expression>)` index.
Results don't change; only the query plan does.

## License

Apache-2.0
