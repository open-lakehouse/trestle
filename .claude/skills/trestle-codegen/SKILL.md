---
name: trestle-codegen
description: Build proto-driven REST services with the Trestle/olai-codegen Rust framework. Use when writing or editing .proto files for a Trestle project, adding endpoints or resources, running `trestle generate` / `just regen`, implementing generated Axum handler traits, wiring generated clients, or wiring the olai-store resource layer. Covers the google.api.resource / google.api.http / google.api.field_behavior / google.api.resource_reference / debug_redact annotations and exactly what code each one generates.
---

# Trestle proto-driven codegen

Trestle turns annotated protobuf into idiomatic Rust: Axum server handler traits,
typed HTTP clients, resource registries, and optional Python/TypeScript/WASM
bindings. The crates involved:

- **`olai-codegen`** — the generator. Reads a compiled proto descriptor, emits code.
- **`olai-trestle`** — the `trestle` CLI (`new` / `generate` / `enrich-openapi`)
  that drives `olai-codegen` and scaffolds projects.
- **`olai-store`** — generic resource store (`Object`, `Association`, `Label`,
  `ResourceRegistry`); generated `labels.rs` plugs into it.
- **`olai-http`** / **`olai-http-wasm`** — HTTP transports the generated clients
  sit on (native cloud client and browser/WASM, respectively).

## Mental model — proto is the source of truth

Every resource type, field role, route, and client method is **derived from
protobuf annotations**. You do not hand-write resource types, routes, request
extractors, or client methods — you annotate `.proto` files and regenerate.

Two rules that follow from this:

1. **Never edit generated code.** Anything under a `gen/` or `_gen/` directory
   (`crates/*/src/gen/`, `crates/*/src/models/_gen/`, `frontend/src/api/`) is
   overwritten on every regeneration. Edits there are lost and cause CI drift
   failures. Change the `.proto`, then regenerate.
2. **Generation is one-way: proto → code.** To change the API surface, edit the
   proto first.

## Conceptual model — flat routes, encoded hierarchies (Unity Catalog style)

Trestle does **not** follow Google AIP's nested-collection URL convention
(`/catalogs/{c}/schemas/{s}/tables`). It follows the **Databricks Unity Catalog**
design: every resource gets its own **flat, top-level collection route**
(`/catalogs`, `/schemas`, `/tables`), and the parent is identified by a field in
the request (e.g. `catalog_name` on a `ListSchemas` request) rather than by a URL
path segment.

Why flat routes:

- **Avoids the N+1 navigation problem.** A client can list or fetch any resource
  directly with a single call, without first resolving every ancestor in the path
  to construct a deeply nested URL.
- **Stable, addressable resources.** Each resource type has one canonical
  collection endpoint regardless of how deep it sits in the conceptual hierarchy.

But resources still form a **logical hierarchy** (the classic
`Catalog → Schema → Table` chain), and Trestle encodes it. The hierarchy is
**discovered**, not declared in one place:

- A child's parent-scoping field (e.g. `catalog_name` on `ListSchemasRequest`)
  carries `google.api.resource_reference = { child_type: "<svc>.io/Schema" }`.
- During analysis the generator scans every service's `List` request for such
  fields where `child_type` equals that service's **own** managed resource type,
  building a global `(parent, child)` parent map across services.
- It then reconstructs the full ancestor chain by depth (`Catalog → Schema →
  Table`), and uses it to emit `parent_label` / `path_names` in
  `RESOURCE_DESCRIPTORS` and resource-scoped client navigation
  (`catalog.create_schema(...)`, `catalog.schema(name)`).

So: **flat HTTP surface, hierarchical resource model.** You declare the hierarchy
edge-by-edge with `resource_reference.child_type` on the parent-scoping field; the
generator assembles the chain. See `references/proto-annotations.md`
(`google.api.resource_reference`) for the exact annotation.

## Workflow

The loop for adding or changing an endpoint:

1. **Edit the `.proto`.** Add/modify a `message` (with `google.api.resource` if
   it's a resource) and an `rpc` (with `google.api.http` for its route). Annotate
   fields with `google.api.field_behavior` and `debug_redact` as appropriate. See
   `references/proto-annotations.md` for what each annotation generates.
2. **Regenerate.** In a scaffolded project this is `just regen` (which runs
   `buf build` then `trestle generate -c trestle.yaml`). Without `just`:
   ```bash
   buf build --as-file-descriptor-set -o api.bin
   trestle generate --config trestle.yaml
   ```
3. **Implement the handler.** The generator emits a handler **trait** with one
   `async fn` per RPC (`async fn create_catalog(&self, request: CreateCatalogRequest,
   context: Cx) -> Result<Catalog>`). Implement that trait on your service struct.
   It does **not** generate the method bodies — that's your business logic.
4. **Wire the router.** The generator emits one Axum handler fn per RPC but does
   **not** build the `Router` — only your app knows the URL prefixes and which
   routes mix in hand-written handlers. Mount the generated `server` module's fns
   onto a `Router` with your handler impl as state.
5. **Use the client.** The generated client exposes a typed method per RPC with
   request builders. The frontend uses the regenerated TypeScript client.

After any proto change, regenerate before building — CI enforces a codegen-drift
check, so stale generated code fails the build.

## What you hand-write vs. generate

| You hand-write (once, then stable) | The generator owns (overwritten) |
|---|---|
| `RequestContext` (impl `FromRequestParts`) | Handler traits, route handler fns |
| `Error` / `Result` types, `parse_error_response` | Request extractors, typed clients + builders |
| Handler trait impls (business logic) | `labels.rs` (`Resource`/`ObjectLabel`, `RESOURCE_DESCRIPTORS`) |
| The `Router` wiring in `main.rs` | Python / TS / WASM bindings |
| Your store backend impl | — |

## References

- **`references/proto-annotations.md`** — the canonical annotation reference:
  each of the six annotations, the proto import to add, an example, and the exact
  generated artifact. **Read this when authoring or reviewing `.proto` files.**
- **`references/field-roles-and-store.md`** — how `field_behavior` and
  `debug_redact` become `FieldRole`s, the `RESOURCE_DESCRIPTORS` static, and
  wiring `ManagedObjectStore` + `SecretManager`. Read when persisting resources.
- **`references/workflow-and-layout.md`** — the four-crate project layout,
  `trestle new`, `trestle.yaml` config knobs, the `buf` → `trestle generate`
  path, and the WASM transport seam. Read when scaffolding or configuring codegen.

## Examples

### Example: add a resource with a create + get endpoint

In `proto/<pkg>/v1/models.proto`:

```proto
import "google/api/field_behavior.proto";
import "google/api/resource.proto";

message Greeting {
  option (google.api.resource) = {
    type: "my-app.io/Greeting"
    pattern: "greetings/{greeting}"
    plural: "greetings"
    singular: "greeting"
  };

  string name = 1 [(google.api.field_behavior) = IDENTIFIER];
  string recipient = 2 [(google.api.field_behavior) = REQUIRED];
  string message = 3 [(google.api.field_behavior) = OUTPUT_ONLY];
}
```

In `proto/<pkg>/v1/service.proto`:

```proto
import "google/api/annotations.proto";
import "google/api/field_behavior.proto";

service GreetingService {
  rpc CreateGreeting(CreateGreetingRequest) returns (Greeting) {
    option (google.api.http) = { post: "/v1/greetings" body: "greeting" };
  }
  rpc GetGreeting(GetGreetingRequest) returns (Greeting) {
    option (google.api.http) = { get: "/v1/{name=greetings/*}" };
  }
}

message CreateGreetingRequest { Greeting greeting = 1 [(google.api.field_behavior) = REQUIRED]; }
message GetGreetingRequest { string name = 1 [(google.api.field_behavior) = REQUIRED]; }
```

Then `just regen` (or `buf build … && trestle generate …`), implement
`GreetingHandler::create_greeting` / `get_greeting` on your service struct, and
mount the generated handler fns onto your `Router`. See
`references/proto-annotations.md` for what `IDENTIFIER` / `REQUIRED` /
`OUTPUT_ONLY` each do to the generated request/response types and store roles.

### Example: mark a secret field for redaction

A field that holds a long-lived secret should never leak through logs or be
stored in the clear. Annotate it `debug_redact`:

```proto
message Connection {
  option (google.api.resource) = {
    type: "my-app.io/Connection" pattern: "connections/{connection}"
    plural: "connections" singular: "connection"
  };

  string name = 1 [(google.api.field_behavior) = IDENTIFIER];
  string client_secret = 2 [debug_redact = true];
}
```

The generator marks `client_secret` as `FieldRole::Sensitive` in
`RESOURCE_DESCRIPTORS`. When the resource is stored through a
`ManagedObjectStore`, sensitive fields are routed to a `SecretManager` (encrypted
into a vault/KMS) rather than the plain properties JSON, and redacted on read.
See `references/field-roles-and-store.md`.
