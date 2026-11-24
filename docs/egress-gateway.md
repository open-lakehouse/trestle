# Egress gateway and secret delivery over SDS

Status: phases 1 (the secret server, [`crates/secret-server`](../crates/secret-server)),
2, 3, 4, 5 (Omnigent), 6 (the workgraph function, in luminescent-lakehouse) and 7 (Rust
services as SDS consumers, without adoption yet) are implemented. The rest is design.
Phases and their done-whens are at the end.

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
  secret is a `401` and never an unauthenticated request. A `header_mutation`
  filter ahead of the injector removes the caller's `authorization` and
  `x-api-key`. The route's `request_headers_to_remove` can't do it: the router
  applies it after the injector, so it would remove the injected header too.
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
  The egress pins an exact patch release, `envoyproxy/envoy:v1.40.2`, and the
  integration test runs against that pin (see [Risks](#risks)).
- **A node id.** SDS over gRPC requires `node.id` and `node.cluster` in the
  bootstrap. Envoy refuses to start without them.

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
  ([below](#consumers)) and for debugging.
- Only `generic_secret` resources. `DeltaSecrets` isn't needed: each consumer
  asks for a handful of names.

On the same sockets it serves a key service (`olai.keys.v1.KeyService`) for key
encryption keys, which no consumer receives. See [Key material](#key-material).

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
  value it had. On shutdown the server ends its open streams itself, so the old
  container stops at once instead of waiting for Docker to kill it.

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

- **Omnigent** (implemented, phase 5) reaches MCP servers through the egress
  gateway. Its server (`omnigent-server:v0.23.0`) reads an MCP registry from
  `OMNIGENT_MCP_REGISTRY`. The omnigent module renders that registry to
  `modules/omnigent/mcp-registry.yaml` and mounts it into the server. The
  registry is built from the `mcp` routes whose consumers include `omnigent`,
  with one service per route:
  - `id`: the route's name.
  - `url: http://egress-gateway:<port><path>`.
  - `auth: none` and `allow_http: true`. The hop to the egress stays inside
    the environment, and the egress does the TLS.
  - `tools`: the route's `tools`, as the allowlist.

  The module reads the routes from its `egress_routes` render global, which
  carries each route's URL and `mcp` but nothing about its credential.
  Omnigent's id and tool rules are checked at plan time
  (`Module::check_egress_route`). Registry calls leave from the Omnigent
  server, so that is the consumer, and a session selects its services by id
  (`mcp_registry_services`). No agent spec, prompt or container holds the key.
  Omnigent's model upstreams keep today's delivery until an upstream needs
  injection.
- **Unity Catalog** uses its native options for its own credentials.
- **Other third-party images** keep today's delivery. There are no launcher
  shims that would fetch a value over SDS and `exec` the real process.
  MLflow's `MLFLOW_CRYPTO_KEK_PASSPHRASE` (minted by the desktop) is such a
  value: MLflow reads it from its environment, so it stays there.
- **Our own Rust services** (implemented, phase 7) read their secrets over SDS
  instead of from their environment.
  - **The client.** `olai_http::sds::SdsCredentialProvider`, behind olai-http's
    `CredentialProvider` trait and its `sds` feature, fetches one named secret
    over the consumer's socket. It calls `FetchSecrets` on first use, then keeps a
    `StreamSecrets` watch open, which ACKs each version with its nonce and swaps
    the cached value on rotation. While the server is gone (it's being recreated,
    say), the provider keeps the last value and reconnects with backoff. A name
    the consumer may not see is `NotFound`. For a bearer token there's
    `BearerAuth::sds(socket, name)`, next to `token_env` and `token_file`. Both use
    the same Envoy v3 protos as the server (`envoy-types`), so neither the default
    nor the wasm build pulls in tonic.
  - **The module.** A module declares what it reads this way in
    `Provides::sds_secrets` (or `Module::sds_secrets_in`, by knobs). Each entry is a
    name the service asks for and the launch variable that holds it. That variable
    must also be one of the module's declared `secrets`, which still says whether
    it's required or minted:

    ```rust
    Provides {
        secrets: vec![DeclaredSecret::required("EXAMPLE_SERVER_KEK", "…")],
        sds_secrets: vec![SdsSecret::new("server-kek", "EXAMPLE_SERVER_KEK")],
        ..Provides::default()
    }
    ```
  - **The plan.** A module with SDS secrets pulls in the egress module, which runs
    the secret server. With no routes, its Envoy runs idle with no listeners. The
    module becomes a consumer like the egress Envoy:
    - It gets a volume `sds-<module>`, which the secret server mounts at
      `/run/trestle-sds/<module>/` and the module's containers at
      `/run/trestle-sds/`.
    - It gets a listener in `secret-server.yaml` that serves it those names.
    - It gets an override file (`modules/egress-gateway/consumers/<module>.yaml`,
      the same one that carries route URLs). The override mounts the volume, sets
      `TRESTLE_SDS_SOCKET=/run/trestle-sds/sds.sock`, and starts the module after
      `secret-server`, so the module's own template doesn't change.
    - Its SDS variables move to the egress module's declared secrets, the same
      way a route's do, and reach `secret-server` alone. The module's fragment
      must not pass them through.
    - `trestle secret rotate` covers them like route credentials: it recreates
      only `secret-server`, and the module's watch picks up the new value without
      a restart.

    The egress Envoy is one consumer of the same list (`EgressConfig::sds_consumers`)
    with its secrets taken from the routes, not a special case.
  - **Adoption** of SDS tokens in our services (mangrove, headwaters,
    `unity-catalog-rs`'s service-to-service tokens) comes next. A KEK such as
    `UC_SERVER_KEK` doesn't go over SDS: it moves to the key service
    ([Key material](#key-material)).

## Key material

The secret server hands out three kinds of material, in two ways:

| Kind | Examples | Who holds it | How it's served |
|---|---|---|---|
| **Tokens** | Bearer tokens, API keys, service-account keys | The consumer, to present to someone else | SDS: the value itself, watched for rotation |
| **Signing keys** | A JWT issuer's private key | The consumer, to sign with locally | SDS today, as a value. Moving them behind a signing API is possible but not planned |
| **KEKs** | `UC_SERVER_KEK` and other envelope-encryption keys | The secret server only | The key service: wrap and unwrap, never the key |

A token has to leave the server: the consumer sends it on. A KEK doesn't. All a
service does with one is wrap and unwrap the data keys (DEKs) of the values it
seals, and that can be done for it. Keeping the KEK in the server means:

- **A compromised service doesn't take its KEK with it.** It can unwrap while it
  runs, and the server can see and refuse it. It leaves with no key that opens
  the database offline, from a backup or after the fact. With SDS the KEK would
  be in its memory and in every heap dump.
- **Rotation is the server's job.** A new version becomes active without a
  restart of the service, old versions keep opening until a rewrap sweep has
  moved every value, and `min_decrypt` then retires them. With a delivered KEK,
  each service would carry every old key and its own rotation logic.
- **One audit point.** Each wrap and unwrap is a call the server can log (by
  consumer, key and version) and later rate-limit.

The cost is a round trip per unwrap and a dependency on the server being up.
Values opened while the server is away need the client's opt-in DEK cache (off by
default, bounded, expiring). New values need the server.

**The key service.** `olai.keys.v1.KeyService` (proto and stubs in
`olai-key-service`, served by `olai-secret-server`) has four RPCs:

- `Wrap(key, dek)` returns `{version, wrapped}`, under the active version unless
  the request names one between `min_decrypt` and the active version.
- `Unwrap(key, version, wrapped)` returns the DEK. It is refused below
  `min_decrypt` and for an unknown version.
- `GetKey(key)` returns `{active_version, min_decrypt_version, aliases}`.
- `WatchKey(key)` streams the same on every change.

It runs on the consumers' SDS sockets with the same caller identity. A consumer
uses only the keys its listener lists (`listeners[].keys[].{name, env}`), and a
key that isn't its own answers `NOT_FOUND`. Each keyring arrives in the server's
environment as JSON: `{versions: {"1": "<base64 of 32 bytes>", …}, active,
min_decrypt, aliases}`. Keyrings never appear in logs or in `Debug` output.

**The wrap format is olai-store's.** A wrap under version `n` of key `k` is
AES-256-GCM with the KEK id `k@vn` as associated data. That is exactly what
`LocalKeyProvider` produces for a KEK of that id. An existing `UC_SERVER_KEK`
(today a `LocalKeyProvider` KEK with the id `kek-1`) is imported as version 1
with the alias `kek-1`. Its blobs keep opening and rewrap to the active version.

**The client.** `olai_key_service::KeyServiceProvider` (feature `client`)
implements olai-store's `KeyProvider`, so `EnvelopeEncryptor` and its `rewrap`
work unchanged. It answers `active_kek_id()` from a cache that a `WatchKey`
stream keeps current, so a rotation reaches a running service. While the server
is being recreated, the provider keeps the versions it last saw and reconnects.

**The module.** A module declares the keys it seals under in `Provides::keys`
(or `Module::keys_in`, by knobs), by name:

```rust
Provides {
    keys: vec!["server-kek".into()],
    ..Provides::default()
}
```

The planner treats it as an SDS consumer. The module gets its `sds-<module>`
socket volume, `TRESTLE_SDS_SOCKET`, and `TRESTLE_KEY_<KEY>` (here
`TRESTLE_KEY_SERVER_KEK=server-kek`), which it passes to
`KeyServiceProvider::connect`. The secret server's listener for the module lists
the key (`keys: [{name: server-kek, env: TRESTLE_KEYRING_<MODULE>__SERVER_KEK}]`).
That variable is derived by the planner and declared, as required, on the
secret server alone, so the module never names it and no other rendered file
does either.

**The CLI.** `trestle key` keeps each keyring in this machine's keychain,
stored on the environment the way `trestle var set --secret` stores a secret:

```bash
# version 1 from an existing key, keeping the id its blobs were sealed under
trestle key import demo/dev unity-catalog-rs/server-kek --from UC_SERVER_KEK --alias kek-1
# a new random version, made active; secret-server is recreated in the running instances
trestle key rotate demo/dev unity-catalog-rs/server-kek
# once the consumer's sweep (rewrap_all, then kek_usage) reports zero blobs below 2
trestle key retire demo/dev unity-catalog-rs/server-kek --below 2 --confirm-swept
# versions, active, min_decrypt and aliases; never material
trestle key show demo/dev unity-catalog-rs/server-kek
```

- **Keyrings are add-only.** No command removes a version's material. `retire`
  only raises `min_decrypt`, so a blob the sweep missed can still be recovered.
  Every write checks that the new keyring keeps each version and alias of the
  stored one, refuses to build on a stored keyring that has lost a version, and
  reads the result back. `var set`, `var unset` and `secret rotate` refuse the
  keyring variables, and `env rm` / `ws rm` keep their keychain entries.
- **`retire` refuses** an `n` above the active version, and it refuses without
  `--confirm-swept`: anything still sealed under a lower version stops opening.
- **Rotation** recreates `secret-server` with the new keyring, as for tokens.
  The module keeps running, and its provider's watch picks up the new active
  version. `KeyHandle::replace` is the later seam for a live swap without the
  recreate.
- **Back the keychain up.** It holds the only copy of each keyring. Losing it
  makes everything sealed under the keyring unreadable. An encrypted
  `trestle key export` is a follow-up ([#471](https://github.com/open-lakehouse/trestle/issues/471)).
- **Not on a team's state yet.** `--secret` values go to each machine's own
  keychain, so two machines would hold different keyrings for the same sealed
  data (a hosted database, say). `trestle key` refuses writes on a team's state
  until there is a shared keyring source.

Next comes adoption in mangrove (K4) and `unity-catalog-rs` (K5). With the
host-backed source (phase 8), keyrings come from the desktop like other values.

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
- **The Omnigent registry is a prototype.** The rendered registry is a small
  YAML, so following its changes is cheap.
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
5. **Omnigent** (implemented). The image bump and the rendered MCP registry.
   Done when an Omnigent agent whose spec holds no credential lists and calls
   the tools of a route's MCP server.
6. **The workgraph function, live** (implemented, in
   [luminescent-lakehouse#230](https://github.com/open-lakehouse/luminescent-lakehouse/pull/230),
   checked live on `preview/pr-230`). In luminescent-lakehouse: per-agent
   `olk_…` keys in the keychain, one route per agent, the desktop showing
   them. Done when an agent in a local environment works on the work graph
   through its route, and its key appears nowhere but the keychain and the
   secret server's environment.
7. **Rust services as SDS consumers** (implemented, without adoption).
   `SdsCredentialProvider` in `olai-http`, module-declared SDS secrets
   (`Provides::sds_secrets`), and then adoption of tokens by our own services. Done when
   a service reads a secret over its socket and the secret is gone from its
   environment: `crates/stack-runtime/tests/live_sds.rs` runs olai-http's
   `sds_probe` as a module's container. It checks that the probe reads its token
   over SDS, that a recreate of `secret-server` reaches it without a restart, and
   that only the secret server's container environment holds the value.
8. **A host-backed source.** The in-environment server pulls from the desktop
   (then possibly the CLI). Done when rotating a key in the desktop reaches
   Envoy without recreating any container.
9. **SPIRE.** The server's caller check moves to SPIFFE IDs over mTLS, with
   multi-env phase 3. Done when the socket volumes are gone and consumers'
   code is unchanged.
10. **Databricks through the egress (optional).** Only for a service principal
    or a static-bearer Databricks MCP service.

KEKs take their own track, next to phases 7 and 8 ([Key material](#key-material)):
K1 is olai-store's rewrap sweep, K2 the key service in `olai-secret-server` and
its `KeyProvider` client in `olai-key-service` (implemented), K3 the topology,
runtime and CLI side, K4 and K5 adoption in mangrove and `unity-catalog-rs`
(importing `UC_SERVER_KEK` as version 1, aliased `kek-1`), and K6 the desktop.
