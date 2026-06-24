# Casper's Ghost Kitchen: a driver check-in app from one proto file

> Blog narrative (DevRel). This is the **story arc + the key design**, not a
> finished post and not a built app — a later session implements it (with
> limited effort) and writes the finalized prose alongside. Every "trestle
> generates X" claim here traces to the working, CI-verified
> `examples/golden-path-app`; the proto + handler sketches live next to this file.

## The setup

Casper's Ghost Kitchen runs delivery-only kitchens — no dining room, just a
loading bay where gig drivers come and go. Operations has a simple, real
problem: **who's here and available right now, and which order did each one
take?** Drivers should *check in* when they arrive, get *assigned an order*, and
*check out* when they leave. Ops wants a live view of the available roster.

It's a small app — a check-in screen for drivers, a roster for ops. Exactly the
kind of thing that's "too small to be worth a whole backend project," and
exactly where the trestle golden path earns its keep: **you describe the domain
once, in protobuf, and get the server, the API, the clients, and a web app from
one command.**

## Model the domain — proto is the source of truth

Two resources. A `Driver` (the check-in lifecycle) and an `Order` (assigned to a
driver — Casper's wants orders tracked *under* the driver delivering them).

```proto
message Driver {
  option (google.api.resource) = {
    type: "caspers.io/Driver" pattern: "drivers/{driver}" plural: "drivers" singular: "driver"
  };
  string name          = 1 [(google.api.field_behavior) = IDENTIFIER];    // server-assigned
  string display_name  = 2 [(google.api.field_behavior) = REQUIRED];
  string vehicle       = 3 [(google.api.field_behavior) = OPTIONAL];
  DriverStatus status  = 4 [(google.api.field_behavior) = OUTPUT_ONLY];    // AVAILABLE | ON_DELIVERY | OFF
  google.protobuf.Timestamp checked_in_at = 5 [(google.api.field_behavior) = OUTPUT_ONLY];
}
```

Each annotation is a *decision the generator acts on*, not documentation:

| Annotation | What Casper's gets from it |
|---|---|
| `google.api.resource` | `Driver` becomes a managed resource: a `RESOURCE_DESCRIPTORS` entry, an `ObjectLabel::Driver`, resource-scoped clients. |
| `field_behavior = IDENTIFIER` | `name` is the server-assigned id (`drivers/{uuid}`) — the check-in screen never sends it. |
| `field_behavior = REQUIRED` | `display_name` gets validated; the client builder takes it as a required arg. |
| `field_behavior = OUTPUT_ONLY` | `status` / `checked_in_at` are server-managed — a driver can't spoof "available." |
| `resource_reference { child_type }` | declares `Order` as a child of `Driver` → `driver.order(name)` navigation in the client. |

The RPCs are just as declarative — `google.api.http` maps each to a REST route,
and the *same* service definition backs the ConnectRPC API:

```proto
service DriverService {
  rpc CheckIn(CheckInRequest)   returns (Driver) { option (google.api.http) = { post: "/v1/drivers" body: "driver" }; }
  rpc GetDriver(GetDriverRequest) returns (Driver) { option (google.api.http) = { get: "/v1/{name=drivers/*}" }; }
  rpc ListDrivers(...) returns (...) { option (google.api.http) = { get: "/v1/drivers" }; }      // ops roster
  rpc CheckOut(CheckOutRequest) returns (Driver) { option (google.api.http) = { post: "/v1/{name=drivers/*}:checkOut" body: "*" }; }
}
```

(Full sketch: `proto/caspers/drivers/v1/{models,service}.proto` next to this file.)

`CheckOut` is worth a callout — it's not CRUD, it's a **state transition**
(`status → OFF`). It's modeled as a custom `:checkOut` verb on the resource,
which is exactly the shape the golden path handles for non-CRUD actions.

## One command

```bash
trestle new caspers-drivers --app databricks-app-rust \
  --runtime buffa \
  --select app.databricks-app-rust.frontend=react \
  --select app.databricks-app-rust.connect=on
cd caspers-drivers && just regen
```

That scaffolds a four-crate workspace (`common` / `server` / `client` /
`frontend`), and `just regen` turns the proto into:

- **buffa model types** (the shared domain structs),
- **REST handler traits + Axum routes** (one per RPC),
- a **buf ConnectRPC service facade** against the same models,
- a **typed Rust client** and a **browser WASM client**,
- the React frontend + the Databricks Apps deploy config.

You've written a proto file and run two commands. Everything below is glue you'd
write anyway — minus the parts that are now generated.

## Implement once, serve twice

Here's the design insight at the center of the golden path. REST and Connect want
**different-shaped handlers**:

| | REST (trestle) | Connect (`protoc-gen-connect-rust`) |
|---|---|---|
| request | owned `CheckInRequest` | `ServiceRequest<'_, …>` (zero-copy view) |
| context | generic `Cx = RequestContext` | `connectrpc::RequestContext` |
| result | `crate::api::Result<T>` | `ServiceResult<impl Encodable<T>>` |
| error | your `api::Error` | `connectrpc::ConnectError` |

One struct can't satisfy both traits — but the *intent* is identical and the real
logic is tiny. The resolution: a **protocol-neutral `DriverCore`** with the
domain methods (`check_in`, `check_out`, `assign_order`), and **two thin
adapters** that each translate one protocol's shapes around the same core call:

```
DriverCore  (the roster + check_in / check_out / assign_order)
   ├── REST adapter     impl DriverHandler       (CoreError → api::Error)
   └── Connect adapter  impl connect DriverService (CoreError → ConnectError)
both mounted on ONE axum listener:
   Router::new().merge(rest).fallback_service(connect.into_axum_service())
```

(Shapes: `handler-core-sketch.md` next to this file.) A quiet but important win:
the buffa models trestle generates for REST already carry the view types the
Connect facade needs, so **Connect reuses the same model crate** — adding the
second protocol is a second adapter, not a second codegen pipeline.

The payoff, proven on the golden-path app and identical here: a driver checks in
over Connect, and ops reads the roster over REST — **same core, one port.**

## The driver web UI

The check-in screen is a React app that imports the **generated browser client**
— compiled to WASM, not a hand-rolled `fetch`:

```tsx
import init, { CaspersDriversClient } from "@/wasm/client";

await init();
const client = new CaspersDriversClient(window.location.origin);   // same-origin
const driver = await client.driver().checkIn({ driver: { displayName, vehicle } });
```

Same-origin means zero env-conditional code: it works behind Vite's dev proxy
locally and behind the Databricks edge in production. The driver taps "I'm
here," the WASM client calls `CheckIn`, and ops' roster (a `ListDrivers` poll)
shows them as `AVAILABLE`. The whole client — types, methods, error classes — is
generated from the same proto.

## What we learned (building the golden path that makes this possible)

This app is small because the path under it was hardened first. The honest
lessons from that build (full blow-by-blow in `examples/JOURNAL.md`):

- **"Proto is the source of truth" actually pays off.** Adding a second protocol
  (ConnectRPC) on top of REST wasn't a rewrite — it was one more generated facade
  over the *same* models and one more thin adapter over the *same* core. The
  domain logic never reshaped.

- **Generated code has to compile against its own scaffold.** The first real
  friction wasn't the fancy stuff — it was the basics: the client crate didn't
  build against the template, `gen` is a reserved word in Rust 2024, the Axum
  extractors were orphaned from the module tree. Closing those is what turned
  "it generates" into "it builds, clean, every time." The tooling is only golden
  if the *default* output is.

- **One client crate, two targets.** The generated client picks its transport at
  compile time with `cfg(target_arch = "wasm32")`: the native cloud client for
  server-side Rust, the browser fetch client for the frontend — from a single
  generated crate. The frontend consuming the *generated* client (not a stub) is
  what makes the end-to-end story real.

- **Guardrails keep the example honest.** A CI job regenerates the reference app,
  runs the post-codegen `clippy --fix` + `fmt`, and fails on any divergence — so
  the thing the blog points at can't silently rot. (A nice side-quest: the build
  proxy lags the registry by weeks for supply-chain safety, so we develop against
  local path-deps and switch to the published versions before pushing.)

## What's next

This pass is the narrative + the design. The follow-up: actually scaffold
`caspers-drivers`, implement the `DriverCore` + the two adapters + the check-in
screen with limited effort, and confirm the live browser round-trip (once the
npm proxy / `olai-http-wasm` publish settle). Beyond that — a real backing store
(Postgres/MLflow), which is its own dedicated effort — and the finalized blog
prose written alongside the working app.
