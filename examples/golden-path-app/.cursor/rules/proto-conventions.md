---
description: Annotations and naming rules the trestle codegen expects in .proto files
---

# Proto conventions

The trestle codegen reads three annotation families:

| Annotation | Purpose |
|-----------|---------|
| `google.api.resource` | Marks a message as a REST resource. Drives URL naming + the resource registry. |
| `google.api.field_behavior` | `REQUIRED`, `OPTIONAL`, `OUTPUT_ONLY`, `IMMUTABLE`, `IDENTIFIER`. Validates payloads + selects which fields the client must supply. |
| `google.api.http` | REST verb + path mapping. Drives Axum route registration + the TS client. |

## Resource skeleton

```proto
message MyResource {
  option (google.api.resource) = {
    type: "golden-path-app.io/MyResource"
    pattern: "myresources/{myresource}"
    plural: "myresources"
    singular: "myresource"
  };

  string name = 1 [(google.api.field_behavior) = IDENTIFIER];
  string display_name = 2 [(google.api.field_behavior) = REQUIRED];
  google.protobuf.Timestamp create_time = 3 [(google.api.field_behavior) = OUTPUT_ONLY];
}
```

## RPC skeleton (REST mapping)

```proto
rpc CreateMyResource(CreateMyResourceRequest) returns (MyResource) {
  option (google.api.http) = { post: "/v1/myresources" body: "my_resource" };
}
rpc GetMyResource(GetMyResourceRequest) returns (MyResource) {
  option (google.api.http) = { get: "/v1/{name=myresources/*}" };
}
```

## Naming rules

- Resource `type` is `<service>.io/<PascalName>`.
- Resource `pattern` and URL path use **plural snake-or-kebab** form
  (`myresources`, not `MyResource`).
- Field names are snake_case in proto, camelCase in JSON (handled by prost-serde).
- Don't reuse field numbers; once an API ships, treat them as immutable.

## After editing a .proto file

```
just regen
```

That command rebuilds `api.bin`, then runs `trestle generate -c trestle.yaml`.
The four output trees regenerate; CI's `codegen-drift` check fails if you forget.