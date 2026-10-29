# Documentation

Design records and deeper explanations for Trestle.

- [Architecture](architecture.md) — the build-time → runtime pipeline, the crate
  map, and the key design decisions (proto-driven, `Label`-generic store, flat
  routing, transport-decoupled clients).
- [Multiple environments](multi-env.md) — running several environments side by
  side as a workspace (ports, project names, the gateway mesh), and the path to
  SPIFFE/SPIRE workload identity.
- [Credential vending](credential-vending.md) — proposed: protocol-neutral crates
  for vended storage credentials (model, client-side `object_store` integration,
  server-side minting) shared by Unity Catalog, Iceberg REST and Open Sharing.

For getting started, see the [top-level README](../README.md). For local
development and the release process, see [`CONTRIBUTING.md`](../CONTRIBUTING.md).
Each crate also has its own README with usage detail:

- [`olai-codegen`](../crates/olai-codegen/README.md) — the generator: annotations,
  config, library API.
- [`olai-store`](../crates/olai-store/README.md) — the resource store traits and
  field-role enforcement.
- [`olai-http`](../crates/olai-http/README.md) — cloud credentials + HTTP client.
- [`olai-http-wasm`](../crates/olai-http-wasm/README.md) — the browser/WASM
  transport.
- [`olai-trestle`](../crates/trestle/README.md) — the CLI: scaffolding and
  template authoring.
