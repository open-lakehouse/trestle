# Egress gateway and secret delivery over SDS

Status: design. Nothing here is implemented yet. Phases and their done-whens
are at the end.

An environment's gateway today is its **ingress**: the Envoy that publishes the
host ports, routes to every service, and drops what a client sends as its own
credential on the routes that hold their own (`strip_client_credentials`).
Credentials reach the services that need them as process environment at launch
([`runtime.md`](runtime.md#decision-credentials-reach-compose-as-process-environment-only)).

That is not enough for agents. An agent in Omnigent that calls an outside
service, such as a work-graph MCP server running as a Neon Function, would
need that service's API key in its spec, its prompt or its container. This
document adds an **egress gateway**: a second Envoy that sits between the
environment's services and the outside, and adds the credential to each
outbound request itself. It also adds the **secret server** that hands Envoy
those credentials over SDS (Envoy's secret discovery service), so that no
value is written to disk. Later, the same server can hand credentials to our
own Rust services.

## Ingress and egress

Inbound and outbound traffic get separate Envoys:

| | Ingress (`envoy`, today) | Egress (`egress-gateway`, new) |
|---|---|---|
| Reached from | the host: published ports, the mesh | the environment's services, on its private network only |
| Published ports | the gateway, admin and dedicated listeners | none |
| Mesh network | joins it when `mesh = true` | never |
| Credentials | holds none; strips the client's own | the only Envoy that holds any; injects them |
| Config | static bootstrap, rendered | static bootstrap, rendered; secrets over SDS |

Keeping them apart means the Envoy a browser reaches never holds a secret, and
the Envoy that holds secrets cannot be reached from outside the environment. A
bug in an ingress route can't turn it into a credential-adding proxy for the
host.

### The egress Envoy

- **One listener per route**, on internal ports from 10001. Envoy's
  `credential_injector` filter has no per-route configuration, so each
  credential needs its own HTTP filter chain, and therefore its own listener.
  A consumer calls `http://egress-gateway:<port><path>`. Ports follow the
  route order in the plan and never reach the host, so they don't take part
  in slots.
- **Outbound TLS** to each upstream, with the cluster shape the ingress already
  uses to front a Neon Function ([`multi-env.md`](multi-env.md#where-services-run-hybrid-environments)):
  `LOGICAL_DNS`, SNI and a certificate check against the system trust store,
  and `auto_host_rewrite`.
- **Credential injection** with `envoy.filters.http.credential_injector`:
  `overwrite: true`, so a caller can't put its own credential in place of the
  injected one, and `allow_request_without_credential: false`, so a missing
  secret is a `401` and never an unauthenticated request. The route also
  removes the caller's `authorization` and `x-api-key` before injecting.
- **The generic credential** (`envoy.http.injected_credentials.generic`) names
  a secret fetched over SDS, the header (`Authorization` by default) and
  `header_value_prefix` (`Bearer `), so the server holds the raw key.
  **OAuth2 client credentials** (`envoy.http.injected_credentials.oauth2`) are
  for upstreams that offer them: Envoy fetches and refreshes the token itself
  and only the client secret comes over SDS.
- **Admin** binds `127.0.0.1` inside the container and is never published. The
  healthcheck (`/ready` over `/dev/tcp`, as for the ingress) runs in the
  container, so nothing else on the network reaches `/config_dump`,
  `/runtime_modify` or `/quitquitquit`. `/config_dump` redacts SDS generic
  secrets anyway (`GenericSecret.secret` is marked `sensitive`, and the
  handler runs `MessageUtil::redact`), but that is a second line, not the
  lock.
- **Its own image pin.** `credential_injector` and both credential extensions
  are `alpha` in Envoy 1.40. The ingress follows `envoyproxy/envoy:v1.40-latest`.
  The egress pins an exact patch release, and the integration test runs against
  that pin (see [Risks](#risks)).

The ingress is unchanged. `strip_client_credentials` keeps its meaning: the
upstream behind that route holds its own credential (MLflow's AI Gateway holds
its provider keys).

## The secret server

`olai-secret-server` is a new crate, a library and a binary, published as the
image `ghcr.io/open-lakehouse/trestle-secret-server:<version>` (multi-arch,
pushed by trestle's release workflow) and pinned in the catalog like any other
image. It runs as the `secret-server` service of the `egress-gateway` module.

It serves the SDS gRPC API from the Envoy v3 protos (`envoy-types`, generated
with prost and tonic 0.14):

- `SecretDiscoveryService.StreamSecrets`, the state-of-the-world stream Envoy
  opens for an `sds_config` with `api_type: GRPC`. The server tracks
  `version_info` and the response nonce per stream, treats a request with the
  same nonce and an error detail as a NACK (logged, not retried with the same
  version), and sends a new version when a value changes.
- `SecretDiscoveryService.FetchSecrets`, the unary call, for the Rust client
  (below) and for debugging.
- Only `generic_secret` resources. `DeltaSecrets` isn't needed: each consumer
  asks for a handful of names.

Its configuration is a rendered, non-secret file,
`modules/egress-gateway/secret-server.yaml`:

```yaml
version: 1
listeners:
  - socket: /run/trestle-sds/egress-gateway/sds.sock
    consumer: egress-gateway
    secrets:
      - name: workgraph-planner          # the name Envoy asks for
        env: WORKGRAPH_PLANNER_API_KEY   # the launch variable that holds it
      - name: workgraph-reviewer
        env: WORKGRAPH_REVIEWER_API_KEY
```

- **Values come from its process environment**, read once at start. A secret
  the file lists but the environment doesn't set fails startup, so the
  instance fails `up --wait` rather than serve `401`s. The runtime already
  refuses to start without a required secret
  ([`runtime.md`](runtime.md#secret-providers-and-sources)), so this is a
  second check.
- **A consumer only ever sees its own names.** A request for any other name
  gets no resource for it on the stream (Envoy waits, then
  `initial_fetch_timeout` fires) and `NOT_FOUND` from `FetchSecrets`. The
  server never logs a value, and its value type has a redacting `Debug`, as
  every credential type in this repository does.
- **Rotation recreates only the server.** `trestle secret rotate` (and the
  desktop) recreate the `secret-server` container with the new launch value
  (`docker compose up -d --no-deps --force-recreate secret-server`). Envoy's
  stream drops and reconnects, the new version arrives, and the egress Envoy
  swaps it without a restart. While the server is down, Envoy keeps the last
  value it had.

The server is generic: it knows listeners, consumers and names, not what the
names are for.

## Decision: phase 1 delivers egress secrets to an in-environment secret server

Two places could hold the values the server hands out: the host's keychain,
read live by a server running on the host, or the launch environment of a
server running inside the environment. Both look attractive for local use.

| | Keychain-backed (host process) | Injected (container in the environment) |
|---|---|---|
| Who runs it | `trestle up` exits once `compose up --wait` returns, so the CLI would need a new long-running daemon, started by the first `up` and stopped by the last `down` (and after a crash or a reboot, by whom?). The desktop could embed it, but the desktop can quit while its instances keep running. | Compose runs it with the instance. It survives the CLI exiting, restarts with the instance, and needs no new process on the host. |
| Rotation | Live: the server reads the keychain again and pushes a new version. | The server container is recreated. Envoy keeps serving and picks up the new version. |
| Where values sit | The keychain only. | The keychain, plus the server container's environment (visible to anyone who can `docker inspect`, as every service's credentials are today). |
| Caller identity | A socket per consumer would live on the host, but Docker Desktop and Colima don't carry host Unix sockets into containers through bind mounts. So callers would reach it over TCP at `host.docker.internal`, and each would need a bootstrap token. Envoy's bootstrap can't read one from its environment, so the token would land in the rendered tree. | A socket per consumer in a named volume. Volumes live in the engine's VM, and sockets work between containers on every engine. |
| CLI and desktop | Two servers, or one that both must find and share. | Identical: both start instances through `olai-stack-runtime`. |

**Phase 1 is injected.** It keeps the existing rule that no value lands in the
rendered tree or the instance directory, it adds nothing to the host, and it
works the same from the CLI and the desktop. The cost is that rotation is a
recreate of one container, and that the values sit in that one container's
environment rather than in every consumer's.

The server stays in the environment in later phases too. What changes is
**where it gets its values**: a host-backed source, desktop first, that the
in-environment server pulls from. That works like the desktop's metadata-service
token endpoint in luminescent-lakehouse: bound on loopback or the Docker
bridge, reached at `host.docker.internal`, behind an unguessable per-instance
path that is the only launch value. Rotation is then live, and Envoy's side
does not change. Whether the CLI gets the same source depends on whether a
host daemon turns out to be worth having for anything else.

This decision adds to the one in `runtime.md`; it doesn't change it. Egress
credentials still reach Compose as process environment, and only the
`secret-server` service lists them.

## Caller identity

**Before SPIRE: one socket per consumer.** The server listens on one Unix
socket per consumer, and decides what a request may see by the socket it came
in on. Each consumer gets a named volume of its own (`sds-<consumer>`), which
the server mounts at `/run/trestle-sds/<consumer>/` and the consumer at
`/run/trestle-sds/`. No consumer can reach another's socket. Peer credentials
(`SO_PEERCRED`) are no help across containers (most images run as root), and
nothing has to be: the mount is the identity.

Envoy reaches its socket through a static cluster with a `pipe` address and
HTTP/2:

```yaml
clusters:
  - name: secret_server
    typed_extension_protocol_options:
      envoy.extensions.upstreams.http.v3.HttpProtocolOptions:
        "@type": type.googleapis.com/envoy.extensions.upstreams.http.v3.HttpProtocolOptions
        explicit_http_config: { http2_protocol_options: {} }
    load_assignment:
      cluster_name: secret_server
      endpoints:
        - lb_endpoints:
            - endpoint: { address: { pipe: { path: /run/trestle-sds/sds.sock } } }
```

**After SPIRE: SPIFFE IDs over mTLS.** Once an environment runs a SPIRE agent
([`multi-env.md`](multi-env.md#networks), phase 3), the server gets an mTLS
listener and identifies a caller by the SPIFFE ID in its SVID, such as
`spiffe://<trust domain>/env/<env>/svc/egress-gateway`. Inside the server,
both resolve to the same thing: a `CallerId` (a consumer name) taken from the
connection, either from the socket it arrived on or from the verified peer
certificate. The policy (consumer → names) doesn't change. Consumers change
only rendered transport configuration: Envoy's `secret_server` cluster moves
from a `pipe` to a TLS address with the SVID it already fetches from the SPIRE
agent over SDS, and the Rust client switches transport inside its provider.
No consumer's code changes.

## The catalog model: injected credentials

Today an endpoint's `RouteOptions` say whether the ingress strips the caller's
credential. Injection is declared the same way, per upstream, with a new type
in `olai-stack-topology`:

```rust
pub enum InjectedCredential {
    /// `Authorization: Bearer <secret>`.
    Bearer { secret: String },
    /// `<name>: <prefix><secret>`.
    Header { name: String, secret: String, prefix: Option<String> },
    /// Envoy's OAuth2 client-credentials grant; only the client secret is a secret.
    OAuth2ClientCredentials { token_url: String, client_id: String, client_secret: String, scopes: Vec<String> },
}
```

`secret` names a variable, as module secrets do. It is never a value.

- **In the catalog**, a module that calls a fixed outside upstream declares it
  as an external endpoint with `.inject_credential(..)`, mirroring
  `.strip_client_credentials()`. The plan gives the module an egress route and
  points its client environment at the route's URL.
- **In `env.toml`**, the environment declares upstreams no module knows about,
  such as one route per agent:

  ```toml
  [selection.egress]
  enabled = true                      # implied by any route

  [[selection.egress.routes]]
  name = "workgraph-planner"
  function = "workgraph"              # the bound branch's function; or `upstream = "https://…"`
  path = "/mcp"
  consumers = ["omnigent"]
  credential = { kind = "bearer", secret = "WORKGRAPH_PLANNER_API_KEY" }
  mcp = { title = "Work graph (planner)", tools = ["read", "add_item", "add_edge", "annotate", "submit"] }
  ```

  A `function` route resolves the upstream to `${<SLUG>_FUNCTION_URL}` and adds
  the function to `Plan::neon_requirements()`, so binding a branch without it
  fails at bind time. `consumers` lists the modules that may call the route.
  The planner gives each of them the route's URL, and refuses an unknown
  module. `mcp` marks the route as an MCP server for consumers that keep an
  MCP registry (Omnigent, below).

A route's secret is declared on the `secret-server` service only
(`Plan::declared_secrets`), so the runtime's missing-secret check covers it,
and no other service's rendered fragment names it. The sentinel tests in
`runtime.md` gain a case: a route's value appears only in the launch
environment, and only `secret-server` references it.

`[selection.egress]` is a new manifest field, so the manifest goes to version
8. An older file has no egress and reads unchanged.

## Consumers

- **Omnigent** reaches its upstreams through the egress gateway. Its server
  reads an MCP registry from `OMNIGENT_MCP_REGISTRY`, and the egress module
  renders it from the `mcp` routes whose consumers include `omnigent`: one
  service per route, `url: http://egress-gateway:<port><path>`, `auth: none`,
  `allow_http: true` (the hop to the egress is inside the environment; the
  egress does the TLS), and the route's `tools` as the allowlist. Registry
  calls leave from the Omnigent server, so that is the consumer, and an
  agent's spec selects its service by id. No agent spec, prompt or container
  holds the key. The registry's Docker entrypoint wiring is newer than the
  pinned `omnigent-server:v0.22.0`, so this needs an image bump. Omnigent's
  model upstreams keep today's delivery until an upstream needs injection.
- **Unity Catalog** uses its native options for its own credentials.
- **Other third-party images** keep today's delivery. There are no launcher
  shims that would fetch a value over SDS and `exec` the real process.
  MLflow's `MLFLOW_CRYPTO_KEK_PASSPHRASE` (minted by the desktop) is such a
  value: MLflow reads it from its environment, so it stays there.
- **Our own Rust services**, later: an `SdsCredentialProvider` in `olai-http`,
  behind the `CredentialProvider` trait, fetches a named secret over the
  consumer's socket (`FetchSecrets`, then a `StreamSecrets` watch for
  rotation). A module declares the secrets it reads this way, and the plan
  gives it a socket volume and the server a listener for it. Its secrets then
  leave its process environment. `unity-catalog-rs`'s `UC_SERVER_KEK` and
  service-to-service tokens are the first candidates.

## The first user: agents and the workgraph function

luminescent-lakehouse runs its agents' work-graph MCP server as the Neon
Function `workgraph`. Its design already expects that "locally, an Envoy route"
adds the credential, and that no agent holds one. Each agent gets one egress
route to the function, injecting that agent's Neon Auth service-account API key
(`olk_…`). The desktop's own apps token is never put on a route: everything
behind the route would get the app-only tools, and an agent could approve its
own plan.

The next candidates for routes are the Neon AI Gateway token, model provider
keys and Databricks bearer tokens.

## The environment graph

Hosted resources are grouped by the cloud that holds them. The group's title
becomes its display name: **Neon**, **Databricks**, **AWS**, **Azure**, and
**Hosted** for a group with no cloud. Today it reads "External cloud · NEON".

- **Neon** holds the branch's Postgres and storage, its functions (Unity
  Catalog, Headwaters, a route's `function`), and its **AI Gateway** when MLflow
  or Omnigent use the `neon` upstream. Today that last one is only a knob.
- **Databricks** is new. It holds the **AI Gateway** (MLflow's and Omnigent's
  `databricks` upstream) and the **sandbox host** that Omnigent can run sessions
  on.
- **The egress gateway** is drawn like the ingress, with an edge from each
  consumer to it and an edge from it to each upstream.

Edges to anything outside the environment say how the connection is
authenticated: `injected` (through the egress gateway), `native` (the
service's own client and configuration, such as a Databricks profile), or
`token` (a credential in the service's environment). The Mermaid export uses
the same titles and labels.

### Databricks through the egress gateway?

Not in the early phases. MLflow's and Omnigent's Databricks integrations are
native: the Databricks SDK, or a CLI profile with user (U2M) OAuth that
refreshes itself. The desktop already keeps those tokens out of the
environment, behind its metadata-service endpoint. `credential_injector`'s
OAuth2 support is the client-credentials grant only, so it can't stand in for a
user login. A route would buy something only for a service principal (M2M), or
for a Databricks MCP service called with a static bearer token. That stays an
optional phase, taken only if such a case shows up.

## Desktop

The desktop starts instances through `olai-stack-runtime`, as the CLI does, with
the same secret providers, so phase 1 needs no code of its own there. It keeps
the agents' API keys in the keychain as local secret vars, shows the
environment's egress routes, and rotates a key by recreating the secret server.
The host-backed source of the later phase starts in the desktop, which already
runs a long-lived endpoint of that shape.

## Risks

- **`credential_injector` is alpha.** Its config or behaviour may change between
  Envoy releases. The egress pins an exact Envoy patch release, and an
  integration test runs that image against the secret server and an echo
  upstream. If the filter breaks or goes away, the fallback is an `ext_proc`
  filter served by the same server, or a small proxy of our own. Either keeps
  the catalog model and the secret server.
- **Values in one container's environment.** `docker inspect` on
  `secret-server` shows them to anyone with access to the Docker engine. That
  is true of every service's credentials today. The host-backed source removes
  it.
- **Direct egress isn't closed.** A container can still reach the internet
  without the egress gateway. The gateway keeps credentials out of agents. It
  doesn't stop an agent calling out. Putting workloads on an `internal: true`
  network, with the egress as the only way out, is a later hardening step.
- **The Omnigent registry is a prototype**, and using it needs an image bump.
  The rendered registry is a small YAML, so following its changes is cheap.
- **One secret server per environment.** If it is down at Envoy's start, the
  egress listeners wait for their secrets until `initial_fetch_timeout` and
  then reject requests (`401`). While it is down later, Envoy keeps the last
  values.
- **A wrong credential on a route.** The route's credential decides what every
  caller behind it can do. The desktop and the docs must never put an apps or
  admin token on an agent's route.

## Phases

1. **The secret server.** `olai-secret-server` (library, binary and image)
   serves `StreamSecrets` and `FetchSecrets` over per-consumer Unix sockets
   from `secret-server.yaml` and its environment. Done when an Envoy at the
   pinned version, in a Docker test, injects a bearer token fetched from it
   into requests to an echo upstream, picks up a rotated value after the
   server is recreated, and a consumer asking on another's socket gets nothing.
2. **The graph.** Cloud groups carry display names, Databricks is a group
   (AI Gateway and sandbox host), the Neon AI Gateway is a node, and edges to
   the outside carry their authentication. Done when the plan goldens, the
   Mermaid export and the env-editor show "Neon" and "Databricks" groups for
   the matching knobs.
3. **The egress gateway in the planner.** `InjectedCredential`,
   `[selection.egress]` (manifest version 8), the `egress-gateway` module with
   its Envoy and `secret-server` services, socket volumes, consumer URLs,
   `neon_requirements` for function routes, and the egress node in the graph.
   Done when a plan with a function route renders an egress Envoy with no
   published port and a loopback admin, and only `secret-server` references
   the route's secret.
4. **Running it.** The runtime scopes route secrets to the secret server,
   `trestle secret rotate` recreates it, and the sentinel tests cover routes.
   Done when `trestle up` with a route against a test upstream gets a `200`
   through the egress, and no value appears in the instance directory.
5. **Omnigent.** The image bump and the rendered MCP registry. Done when an
   Omnigent agent whose spec holds no credential lists and calls the tools of
   a route's MCP server.
6. **The workgraph function, live.** In luminescent-lakehouse: per-agent
   `olk_…` keys in the keychain, one route per agent, the desktop showing
   them. Done when an agent in a local environment works on the work graph
   through its route, and its key appears nowhere but the keychain and the
   secret server's environment.
7. **Rust services as SDS consumers.** `SdsCredentialProvider` in `olai-http`,
   module-declared SDS secrets, and adoption by our own services. Done when a
   service reads a secret over its socket and the secret is gone from its
   environment.
8. **A host-backed source.** The in-environment server pulls from the desktop
   (then possibly the CLI). Done when rotating a key in the desktop reaches
   Envoy without recreating any container.
9. **SPIRE.** The server's caller check moves to SPIFFE IDs over mTLS, with
   multi-env phase 3. Done when the socket volumes are gone and consumers'
   code is unchanged.
10. **Databricks through the egress (optional).** Only for a service principal
    or a static-bearer Databricks MCP service.
