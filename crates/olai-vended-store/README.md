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

The credential model, storage locations and wire adapters live in
[`olai-storage-credentials`](../olai-storage-credentials). The core builds for
`wasm32-unknown-unknown`.

See the [credential vending design record](../../docs/credential-vending.md).
