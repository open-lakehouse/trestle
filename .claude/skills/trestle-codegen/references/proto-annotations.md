# Proto annotation reference

The Trestle generator (`olai-codegen`) reads **standard Google API extensions
only** — there are no custom Trestle proto extensions. Add the relevant
`google/api/*.proto` imports and depend on `buf.build/googleapis/googleapis` (or
vendor the protos) so the extensions resolve.

The six annotations the generator acts on, and what each produces:

| Annotation | Ext. field | What it drives |
|---|---|---|
| `google.api.http` | 72295728 | RPC → HTTP verb + URL pattern → Axum route handler + client method + TS client |
| `google.api.resource` | 1053 | Marks a message as a managed resource → `Resource`/`ObjectLabel` enums + `Label` impl + `RESOURCE_DESCRIPTORS` in `labels.rs` |
| `google.api.field_behavior` | 1052 | Per-field role: request/response inclusion, builder required-ness, store field roles |
| `google.api.resource_reference` | 1055 | Parent/child hierarchy via `child_type` → scoped clients + hierarchical names |
| `debug_redact` | FieldOptions field 16 | `FieldRole::Sensitive` → routed to `SecretManager`, redacted on read |
| `gnostic.openapi.v3.operation` | 1143 | Operation metadata → method naming + OpenAPI enrichment (`trestle enrich-openapi`) |

Extension numbers are defined in `crates/olai-codegen/src/parsing/mod.rs`; the
field-behavior values and `debug_redact` extraction live in
`crates/olai-codegen/src/parsing/message.rs`.

---

## `google.api.resource`

Import: `google/api/resource.proto`. Applied as a **message option**.

Marks a message as a managed resource type. Drives URL naming, the `Resource`
enum, the `ObjectLabel` discriminant, the `olai_store::Label` impl, and the
`RESOURCE_DESCRIPTORS` static used by the store layer.

```proto
message Catalog {
  option (google.api.resource) = {
    type: "example.io/Catalog"          // "<service>.io/<PascalName>"
    pattern: "catalogs/{catalog}"        // hierarchical path template
    plural: "catalogs"                   // collection segment
    singular: "catalog"                  // singular form; seeds ObjectLabel variant
  };
  // ... fields ...
}
```

Fields of the descriptor:

- **`type`** — globally unique resource type id, conventionally
  `<service>.io/<PascalName>`. This is the value `resource_reference.child_type`
  points at to declare hierarchy.
- **`pattern`** — slash-separated path template; nested patterns
  (`catalogs/{catalog}/schemas/{schema}`) declare a child resource.
- **`plural`** / **`singular`** — used for collection vs. singular naming. The
  singular seeds the `ObjectLabel` enum variant.
- **`name_field`** (optional) — points the leaf name at a field other than
  `name` (e.g. `full_name` for a dot-joined composite). When set (to anything but
  `full_name`), the resource is treated as hierarchical.

**Generates** (in `<output_models>/_gen/labels.rs`):

```rust
pub enum Resource { Catalog(/* … */), Schema(/* … */) }   // one variant per resource
pub enum ObjectLabel { Catalog, Schema }                  // discriminant
impl ::olai_store::Label for ObjectLabel { /* … */ }
impl From<Catalog> for Resource { /* … */ }               // + TryFrom back
pub static RESOURCE_DESCRIPTORS: &[ResourceTypeDescriptor<ObjectLabel>] = &[ /* … */ ];
```

Requires `generate_resource_enum: true` in `trestle.yaml` (which pulls in
`olai-store`). A message **without** this annotation is generated as a plain
message type — it can still be an RPC request/response, just not a stored resource.

---

## `google.api.http`

Import: `google/api/annotations.proto`. Applied as a **method option**.

Maps each RPC to an HTTP verb and URL pattern.

```proto
service CatalogService {
  rpc CreateCatalog(CreateCatalogRequest) returns (Catalog) {
    option (google.api.http) = { post: "/catalogs" body: "*" };
  }
  rpc GetCatalog(GetCatalogRequest) returns (Catalog) {
    option (google.api.http) = { get: "/catalogs/{name}" };
  }
  rpc UpdateCatalog(UpdateCatalogRequest) returns (Catalog) {
    option (google.api.http) = { patch: "/catalogs/{name}" body: "*" };
  }
  rpc DeleteCatalog(DeleteCatalogRequest) returns (DeleteCatalogResponse) {
    option (google.api.http) = { delete: "/catalogs/{name}" };
  }
}
```

- **Verb** — `get` / `post` / `put` / `patch` / `delete` with the URL pattern.
- **Path params** — `{name}` binds to the request field `name`. Segment bindings
  like `{name=greetings/*}` are supported; dotted nested paths are not yet.
- **`body`** — `"*"` binds the whole request as the body; a field name (e.g.
  `body: "greeting"`) binds just that field.

**Generates** per RPC:
- a method on the **handler trait** (`async fn create_catalog(&self, request:
  CreateCatalogRequest, context: Cx) -> Result<Catalog>`) — you implement it;
- an Axum **route handler fn** in the sibling `server` module that extracts path
  params + body into the request type and calls your handler (you still mount it
  on a `Router` yourself);
- a typed **client method** + request builder;
- the corresponding **TypeScript client** method when TS output is configured.

Standard CRUD verbs are recognized as List/Create/Get/Update/Delete; RPCs that
don't fit the resource CRUD shape (e.g. `post: "/catalog-tokens"`) are emitted as
custom methods.

---

## `google.api.field_behavior`

Import: `google/api/field_behavior.proto`. Applied as a **field option**;
repeatable.

Recognized values (the generator decodes all of them; the ones with codegen
effects are noted):

| Value | Meaning / effect |
|---|---|
| `REQUIRED` | Caller must supply it; surfaced as a required builder argument. |
| `OPTIONAL` | Explicitly optional (emphasis). |
| `OUTPUT_ONLY` | Response-only; excluded from request building. Combined with a known managed name (`created_at`/`updated_at`/`created_by`/`updated_by`) → `FieldRole::Managed`. |
| `INPUT_ONLY` | Request-only; excluded from response parsing. |
| `IMMUTABLE` | Settable once, at create time. |
| `IDENTIFIER` | The resource's identifier field → `FieldRole::Identifier` (maps to `Object.id` in the store). |
| `UNORDERED_LIST` | Repeated field order not guaranteed. |
| `NON_EMPTY_DEFAULT` | Non-empty default when unset. |

```proto
string name        = 1 [(google.api.field_behavior) = IDENTIFIER];
string recipient   = 2 [(google.api.field_behavior) = REQUIRED];
string message     = 3 [(google.api.field_behavior) = OUTPUT_ONLY];
int64  created_at  = 4 [(google.api.field_behavior) = OUTPUT_ONLY];   // → Managed
```

**Convention for store-backed resources:** annotate the resource's identifier
field (usually `name`) with `IDENTIFIER`. This is what maps it to `Object.id` and
produces `FieldRole::Identifier`. (Note: the in-repo `proto/example_*.proto`
fixtures predate this convention and annotate `name` as `OUTPUT_ONLY`, which
yields `FieldRole::Data` — prefer `IDENTIFIER` for new resources you intend to
persist.) See `references/field-roles-and-store.md` for the full role mapping.

---

## `google.api.resource_reference`

Import: `google/api/resource.proto`. Applied as a **field option**, typically on
the parent-scoping field of a `List`/`Create` request.

Declares resource relationships. Two forms:

- **`child_type: "<svc>.io/Child"`** — this field names a parent container for the
  child resource. This is what establishes the parent → child hierarchy.
- **`type: "<svc>.io/Resource"`** — this field directly references a resource of
  that type.

```proto
message ListSchemasRequest {
  // child_type = this service's OWN managed type (Schema). This declares
  // "catalog_name identifies the parent of a Schema", i.e. Catalog → Schema.
  string catalog_name = 1 [
    (google.api.field_behavior) = REQUIRED,
    (google.api.resource_reference) = { child_type: "example.io/Schema" }
  ];
  int32 max_results = 2;
  string page_token = 3;
}
```

### How the hierarchy is discovered

Trestle uses **flat, top-level routes** for every resource (Unity Catalog style:
`/schemas`, not `/catalogs/{c}/schemas`) — see the conceptual section in
`SKILL.md`. Because the hierarchy is *not* in the URL, it is reconstructed from
these `resource_reference` annotations:

1. The generator scans every service's **`List` request** for fields carrying a
   `child_type`.
2. It records an edge only when `child_type` equals **that service's own managed
   resource type**. So `catalog_name` (child_type = Schema) on `SchemaService`'s
   `ListSchemasRequest` records the edge `Catalog → Schema`; `catalog_name`
   (child_type = Table) and `schema_name` (child_type = Table) on `TableService`
   record `Catalog → Table` and `Schema → Table`.
3. From these edges it reconstructs the depth-ordered chain
   (`Catalog → Schema → Table`).

**Convention:** put `resource_reference { child_type: <this service's resource> }`
on the parent-scoping field of the child's `List` (and `Create`) request, one edge
per parent. The leaf is the immediate parent; deeper ancestors are inferred by
chaining.

**Generates:** `parent_label` and `path_names` in `RESOURCE_DESCRIPTORS`, and
resource-scoped clients — e.g. a `CatalogClient` gains `create_schema(...)` /
`schema(name)` navigation, with the parent path component (`catalog_name`)
supplied by the scoped client rather than by the caller.

---

## `debug_redact`

This is a core protobuf field option (`google.protobuf.FieldOptions` field 16),
not a `google.api.*` extension — no extra import is needed. Set it `true` on
fields holding secrets.

```proto
message Connection {
  option (google.api.resource) = {
    type: "example.io/Connection" pattern: "connections/{connection}"
    plural: "connections" singular: "connection"
  };
  string name          = 1 [(google.api.field_behavior) = IDENTIFIER];
  string client_secret = 2 [debug_redact = true];
}
```

**Generates:** `FieldRole::Sensitive` for that field in `RESOURCE_DESCRIPTORS`.
At runtime, a `ManagedObjectStore` routes sensitive fields to a `SecretManager`
(encrypted into a vault/KMS) instead of the plain properties JSON, and redacts
them on read. See `references/field-roles-and-store.md`.

> This is the proto-level companion to the hand-written redaction rule for
> Rust credential structs (hand-write `Debug` rendering `<redacted>`; never
> `#[derive(Debug)]` on a secret-bearing type). The proto annotation governs
> *stored/serialized* secrets; the `Debug` rule governs *in-memory* ones.

---

## `gnostic.openapi.v3.operation`

OpenAPI operation metadata (operationId, tags, summary, …). The generator reads
the operation and prefers its `operation_id` (snake-cased) for method naming; it
is also consumed by `trestle enrich-openapi` to enrich generated OpenAPI specs.
Optional — omit it and method names are derived from the RPC name.

---

## Naming rules (quick reference)

- Resource `type` is `<service>.io/<PascalName>`.
- Resource `pattern` and URL paths use **plural snake/kebab** form (`catalogs`,
  not `Catalog`).
- Proto field names are `snake_case`; JSON is `camelCase` (handled by the serde
  layer).
- Never reuse field numbers; once an API ships, treat them as immutable.
- After any proto edit, **regenerate** (`just regen`, or `buf build … && trestle
  generate …`) before building — CI fails on codegen drift.
