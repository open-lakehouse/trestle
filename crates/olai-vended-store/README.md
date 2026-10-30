# olai-vended-store

`object_store` stores backed by vended cloud storage credentials: the short-lived,
prefix-scoped credentials that Unity Catalog, the UC Delta API, Iceberg REST and Open
Sharing hand to clients.

- **`CredentialSource`**: vends a fresh `VendedCredential` for one securable at one
  access level. A refresh replays exactly that request, so it never widens the grant.
- **`RefreshingCredentialProvider`** (features `aws`, `azure`, `gcp`): an object_store
  `CredentialProvider` over a source. It is seeded with the credential the store was
  built from, refreshes single-flight, and re-vends a configurable margin (5 minutes by
  default) before expiry. A failed vend is not cached.

- **`build_store`**, **`RoutingObjectStore`**, **`presign`**: one builder for S3, R2,
  Azure, Azurite, GCS, local and in-memory stores; routing by longest prefix; presigned
  URLs that never outlive their credential.

Vending protocols are feature-gated, native-only modules with hand-written clients:

| Feature | Server | Entry point |
|---|---|---|
| `unity` | Unity Catalog REST (tables, volumes, paths) | `unity::UnityCatalog`, `uc://` and `vol+dbfs:` URLs |
| `delta-api` | UC Delta API (catalog-managed tables) | `unity_delta_api` on `unity` |
| `iceberg` | Iceberg REST (`loadTable` + `credentials`) | `iceberg::IcebergCatalog` (builder only) |
| `sharing` | Delta / Open Sharing (volumes, agent skills) | `sharing::OpenSharing`, `sharing:///` URLs |
| `neon` | Neon branch storage (owner tooling; not downscoped) | `neon::NeonStorage` (builder only) |

`parse_url_opts` builds a store from any enabled protocol's URL reference.

The credential model, storage locations and wire adapters live in
[`olai-storage-credentials`](../olai-storage-credentials). The core builds for
`wasm32-unknown-unknown`.

See the [credential vending design record](../../docs/credential-vending.md).
