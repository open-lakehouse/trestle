# Documentation

Design records and deeper explanations for Trestle.

- [Architecture](architecture.md): the build-time to runtime pipeline, the crate
  map, and the key design decisions (proto-driven, `Label`-generic store, flat
  routing, transport-decoupled clients).
- [Multiple environments](multi-env.md): running several environments side by
  side as a workspace (ports, project names, the gateway mesh), and the path to
  SPIFFE/SPIRE workload identity.
- [Credential vending](credential-vending.md): protocol-neutral crates for vended
  storage credentials (model, client-side `object_store` integration, server-side
  minting), shared by Unity Catalog, Iceberg REST and Open Sharing. Accepted, with
  most phases landed.
- [crates.io proxy delay](crates-proxy.md): building the sibling repos against a
  fresh trestle or mangrove release while the proxy still hides it, with the
  `scripts/crates-proxy/` tooling.

For getting started, see the [top-level README](../README.md). For local
development and the release process, see [`CONTRIBUTING.md`](../CONTRIBUTING.md).
Each crate also has its own README with usage detail:

- [`olai-codegen`](../crates/olai-codegen/README.md): the generator, covering
  annotations, config, and the library API.
- [`olai-store`](../crates/olai-store/README.md): the resource store traits and
  field-role enforcement.
- [`olai-http`](../crates/olai-http/README.md): cloud credentials and the HTTP client.
- [`olai-http-wasm`](../crates/olai-http-wasm/README.md): the browser/WASM transport.
- [`olai-trestle`](../crates/trestle/README.md): the CLI, covering scaffolding and
  template authoring.
