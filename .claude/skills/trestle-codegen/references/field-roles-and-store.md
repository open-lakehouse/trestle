# Field roles and the olai-store layer

When `generate_resource_enum: true`, the generator emits a `RESOURCE_DESCRIPTORS`
static into `labels.rs`. Each resource type lists its fields, each tagged with a
**`FieldRole`** derived from proto annotations. `olai-store`'s
`ManagedObjectStore` reads these descriptors and enforces the roles at the
storage boundary.

## How annotations map to `FieldRole`

The mapping (from `crates/olai-codegen/src/codegen/resources.rs`, enforced by
`crates/olai-store/src/registry.rs`), applied per field in this order:

| Condition on the proto field | `FieldRole` | Store behaviour |
|---|---|---|
| `field_behavior = IDENTIFIER` | `Identifier` | Stripped on write; maps to `Object.id`; injected back on read |
| `debug_redact = true` | `Sensitive` | Routed to the `SecretManager`; redacted on read |
| `field_behavior = OUTPUT_ONLY` **and** name ∈ {`created_at`, `updated_at`, `created_by`, `updated_by`} | `Managed` | Stripped on write; injected by the store on read |
| anything else | `Data` | Stored as-is in the properties JSON |

Notes:

- `OUTPUT_ONLY` alone does **not** make a field `Managed` — the name must also be
  one of the four known managed names. An `OUTPUT_ONLY` field with any other name
  (e.g. a server-computed `message` or `full_name`) is `Data`.
- The role precedence is `Identifier` → `Sensitive` → `Managed` → `Data`: a field
  marked `IDENTIFIER` is `Identifier` even if other annotations are present.

Example generated descriptor:

```rust
pub static RESOURCE_DESCRIPTORS: &[ResourceTypeDescriptor<ObjectLabel>] = &[
    ResourceTypeDescriptor {
        label: ObjectLabel::Catalog,
        fields: &[
            ResourceFieldDescriptor { name: "name",       role: FieldRole::Identifier },
            ResourceFieldDescriptor { name: "comment",    role: FieldRole::Data },
            ResourceFieldDescriptor { name: "created_at", role: FieldRole::Managed },
        ],
        path_names: &["name"],
        parent_label: None,
    },
    // ...
];
```

## The store types

From `crates/olai-store/src/`:

- **`Label`** — type-safe resource discriminant. The generated `ObjectLabel` enum
  implements it.
- **`Object<L>`** — the untyped interchange node: `id: Uuid`, `label: L`,
  `name: ResourceName`, `properties: Option<serde_json::Value>`, timestamps.
- **`Association<L>`** — a directed edge between objects.
- **`ResourceName`** — a slash-separated hierarchical key, e.g.
  `"catalogs/my-catalog/schemas/my-schema"`.
- **`ResourceRegistry<L>`** — runtime lookup over `RESOURCE_DESCRIPTORS`
  (`sensitive_field_names`, `managed_field_names`, `identifier_field_name`,
  `parent_label`, `path_names`).

### Store traits

Read and write are split so read-only callers depend on the narrower trait
(`crates/olai-store/src/store.rs`):

```text
ObjectStoreReader<L>        get, get_by_name, list
ObjectStore<L>            + create, update, delete
AssociationStoreReader<L>   list_associations
AssociationStore<L>       + add, remove
```

Implement these (`#[async_trait]`) over your backend of choice. The bundled store
is for bootstrapping/prototypes — back the traits with a production engine for
real workloads.

## Wiring `ManagedObjectStore`

`ManagedObjectStore<L, S, M>` wraps any `ObjectStore` and enforces the field
roles. Build a registry from the generated static, then wrap your backend:

```rust
use olai_store::{ManagedObjectStore, ResourceRegistry};

let registry = ResourceRegistry::from_static(&RESOURCE_DESCRIPTORS);

// Without secret storage: Sensitive fields are STRIPPED but not persisted anywhere.
let store = ManagedObjectStore::new(backend, registry);

// With secret storage: Sensitive fields are encrypted into your vault/KMS.
let store = ManagedObjectStore::with_secrets(backend, my_secret_manager, registry);
```

On create/update the managed store strips `Identifier` and `Managed` fields (the
store owns those), routes `Sensitive` fields to the `SecretManager`, and stores
the rest as properties. On read it injects `Identifier`/`Managed` fields back and
redacts `Sensitive` fields — use the secret-aware read path
(`get_with_secrets`) when you genuinely need the decrypted values.

`SecretManager` is the trait for encrypting `Sensitive` fields; the default
`NoSecrets` strips them without storing them. See the `olai-store` rustdoc for
the full API.
