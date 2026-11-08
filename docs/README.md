# Documentation

Design records and deeper explanations for Trestle.

- [Architecture](architecture.md): the build-time to runtime pipeline, the crate
  map, and the key design decisions (proto-driven, `Label`-generic store, flat
  routing, transport-decoupled clients).
- [Multiple environments](multi-env.md): running several environments side by
  side as a workspace (ports, project names, the gateway mesh), and the path to
  SPIFFE/SPIRE workload identity.
- [Running environments](runtime.md): `olai-stack-runtime`'s Compose engine, what
  the Docker engines print (captured as fixtures), and how credentials reach
  Compose; and the state store the CLI and the desktop app share.
- [Credential vending](credential-vending.md): protocol-neutral crates for vended
  storage credentials (model, client-side `object_store` integration, server-side
  minting), shared by Unity Catalog, Iceberg REST and Open Sharing. Accepted, with
  most phases landed.
- [Neon from trestle](neon.md): what the `neon` CLI prints and does for branches,
  Postgres, object storage and functions (the WP2 spike), behind the Neon branch
  source.
- [crates.io proxy delay](crates-proxy.md): building the sibling repos against a
  fresh trestle or mangrove release while the proxy still hides it, with the
  `scripts/crates-proxy/` tooling.

For getting started, see the [top-level README](../README.md). For local
development and the release process, see [`CONTRIBUTING.md`](../CONTRIBUTING.md).
Each crate also has its own README with usage detail:

- [`olai-codegen`](../crates/olai-codegen/README.md): the generator, covering
  annotations, config, and the library API. Its internals are in
  [`codegen-design.md`](../crates/olai-codegen/docs/codegen-design.md).
- [`olai-store`](../crates/olai-store/README.md): the resource store traits and
  field-role enforcement.
- [`olai-http`](../crates/olai-http/README.md): cloud credentials and the HTTP client.
- [`olai-http-wasm`](../crates/olai-http-wasm/README.md): the browser/WASM transport.
- [`olai-auth`](../crates/olai-auth/README.md): bearer-token authentication and
  authorization for axum services.
- [`olai-storage-credentials`](../crates/olai-storage-credentials/README.md),
  [`olai-vended-store`](../crates/olai-vended-store/README.md) and
  [`olai-credential-vending`](../crates/olai-credential-vending/README.md): the
  credential vending model, client-side stores and server-side minting.
- [`olai-stack-topology`](../crates/stack-topology/README.md): the environment
  planner and renderer behind `trestle env`.
- [`olai-stack-runtime`](../crates/stack-runtime/README.md): running environments
  on Docker Compose.
- [`olai-stack-state`](../crates/stack-state/README.md): the shared state store.
- [`olai-trestle`](../crates/trestle/README.md): the CLI, covering scaffolding and
  template authoring.
