# olai-secret-server

A secret discovery service (SDS) server. It hands secrets to Envoy, and later to other
consumers, over Envoy's v3 `SecretDiscoveryService`, so that no secret value is written
into a rendered config file. It is the `secret-server` of trestle's egress gateway (see
[`docs/egress-gateway.md`](../../docs/egress-gateway.md)), but it knows nothing about
gateways: it knows listeners, consumers and secret names.

- One Unix socket per consumer. The socket a connection arrives on decides what it may
  see.
- Values come from the server's process environment, read once at startup. The
  configuration file only names variables.
- `StreamSecrets` (state of the world) and `FetchSecrets`, `generic_secret` resources
  only. `DeltaSecrets` is not implemented.

It ships as a library, a binary (`olai-secret-server`) and the image
`ghcr.io/open-lakehouse/trestle-secret-server:<version>` (linux/amd64 and linux/arm64).

## Configuration: `secret-server.yaml`

```yaml
version: 1
listeners:
  - socket: /run/trestle-sds/egress-gateway/sds.sock
    consumer: egress-gateway
    secrets:
      - name: workgraph-planner          # the name the consumer asks for
        env: WORKGRAPH_PLANNER_API_KEY   # the variable that holds the value
      - name: workgraph-reviewer
        env: WORKGRAPH_REVIEWER_API_KEY
```

| Field | Meaning |
|---|---|
| `version` | Must be `1`. |
| `listeners[].socket` | The Unix socket the server binds for this consumer. Its directory is created if missing, and a socket left by an earlier run is replaced. The socket is mode `0666`: the volume it lives on, not its owner, is what restricts it to one consumer. |
| `listeners[].consumer` | The consumer every connection on this socket is answered for. Unique across listeners. |
| `listeners[].secrets[].name` | A resource name the consumer may ask for, such as the `name` in Envoy's `sds_config` secret. Unique within the listener. Two consumers can each have a secret with the same name. |
| `listeners[].secrets[].env` | The environment variable that holds the value. |

Unknown fields are an error. So are duplicate consumers, duplicate sockets, and a secret
listed twice for one consumer.

**Every listed variable must be set and non-empty at startup.** Otherwise the server exits
non-zero with one message that lists every missing variable, by variable, secret and
consumer (never a value). Under Compose the instance then fails `up --wait`, instead of
starting and answering with `401`s.

The binary reads the file from `--config` or `SECRET_SERVER_CONFIG` (default
`/etc/trestle/secret-server.yaml`), and logs to stderr with `RUST_LOG` filtering (default
`info`).

## What a consumer sees

A connection's consumer is a `CallerId`, taken from the connection, never the request. Today
it is the listener's consumer. With SPIRE, an mTLS listener will resolve the SPIFFE ID of
the verified peer certificate to the same `CallerId` (`CallerResolver` is the seam), and
the policy stays as it is.

- **StreamSecrets.** The server answers a request with the secrets that are both requested
  and allowed. A name the consumer may not see is never sent. If none of the requested
  names are allowed, nothing is sent at all, so Envoy waits until its
  `initial_fetch_timeout`, and a listener with
  `allow_request_without_credential: false` answers `401`. An empty name list is not a
  wildcard.
  - Each response carries `version_info` `<boot id>-<n>` and a nonce, both per stream. The
    boot id is random for each process, so a restarted server never repeats a version.
  - A request that echoes the last nonce without an error is an ACK. The server sends
    again only if the requested names changed.
  - An echoed nonce with `error_detail` is a NACK. It is logged, and that version is not
    sent again.
  - A request with an older nonce is ignored.
- **FetchSecrets.** Every requested name must be allowed. Otherwise the call fails with
  `NOT_FOUND`, and the message is the same for a name nobody holds and a name another
  consumer holds.

Logs name consumers, secrets, versions and nonces. They never contain a value, and
`SecretValue`'s `Debug` prints `<redacted>`.

## Rotation

Recreate the server with the new value in its environment, for example
`docker compose up -d --no-deps --force-recreate secret-server`. Envoy's stream drops and
reconnects, the new version arrives, and Envoy swaps the secret without a restart. While
the server is down, Envoy keeps the last value it had. olai-http's SDS client
(`SdsCredentialProvider`) behaves the same way.

On `SIGTERM` the server ends every open `StreamSecrets` stream with `UNAVAILABLE` before its
graceful shutdown, which would otherwise wait on them until Docker kills the container. So
a recreate takes as long as the new container takes to start.

## The Envoy side

Give each consumer its own named volume. The server mounts it at
`/run/trestle-sds/<consumer>/`, and the consumer mounts it at `/run/trestle-sds/`. Envoy
reaches the socket through a static cluster with a `pipe` address and HTTP/2. Its bootstrap
must set `node.id` and `node.cluster`, which SDS over gRPC requires:

```yaml
node: { id: egress-gateway, cluster: egress-gateway }
static_resources:
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

A listener injects a bearer token with `credential_injector` and the generic credential.
The caller's own `authorization` and `x-api-key` are removed by a `header_mutation` filter
placed before the injector. A route's `request_headers_to_remove` runs in the router, after
injection, and would remove the injected header too.

```yaml
http_filters:
  - name: envoy.filters.http.header_mutation
    typed_config:
      "@type": type.googleapis.com/envoy.extensions.filters.http.header_mutation.v3.HeaderMutation
      mutations:
        request_mutations: [{ remove: authorization }, { remove: x-api-key }]
  - name: envoy.filters.http.credential_injector
    typed_config:
      "@type": type.googleapis.com/envoy.extensions.filters.http.credential_injector.v3.CredentialInjector
      overwrite: true
      allow_request_without_credential: false
      credential:
        name: envoy.http.injected_credentials.generic
        typed_config:
          "@type": type.googleapis.com/envoy.extensions.http.injected_credentials.generic.v3.Generic
          header: Authorization
          header_value_prefix: "Bearer "
          credential:
            name: workgraph-planner
            sds_config:
              resource_api_version: V3
              initial_fetch_timeout: 5s
              api_config_source:
                api_type: GRPC
                transport_api_version: V3
                grpc_services: [{ envoy_grpc: { cluster_name: secret_server } }]
  - name: envoy.filters.http.router
    typed_config:
      "@type": type.googleapis.com/envoy.extensions.filters.http.router.v3.Router
```

`credential_injector` is alpha, so the egress gateway pins an exact Envoy release:
`envoyproxy/envoy:v1.40.2`. `/config_dump` redacts generic secrets. The Docker test checks
that too.

## Image

```bash
docker build -f crates/secret-server/Dockerfile -t trestle-secret-server .
```

Build it from the workspace root. Where crates.io is only reachable through a mirror, pass a
Cargo config with the source replacement as a build secret:
`--secret id=cargo-config,src=$HOME/.cargo/config.toml`.

The image is distroless and runs as root. The consumer volumes are mounted per consumer, so
the image can't pre-own them for another user.

## Tests

- `cargo test -p olai-secret-server`: config, policy, the stream state machine, and SDS
  over real Unix sockets with a tonic client.
- `cargo test -p olai-secret-server --features emulator-it --tests`: Envoy `v1.40.2` in
  Docker against the server image and an echo upstream. It checks that:
  - the token is injected, replacing the caller's own credentials;
  - a second consumer's socket never yields the first consumer's secret;
  - `/config_dump` holds no value;
  - a recreated server's new value reaches the running Envoy.

  Set `SECRET_SERVER_IMAGE` to use a prebuilt image; otherwise the test builds one. With
  colima, set `DOCKER_HOST=unix://$HOME/.colima/default/docker.sock`.
