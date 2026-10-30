# olai-credential-vending

Server-side minting of downscoped cloud storage credentials: the short-lived,
prefix-scoped credentials that Unity Catalog, Iceberg REST and Open Sharing servers hand
to clients.

- **`RegisteredCredential`**: the long-lived principal a host registers (an IAM role, an
  Azure service principal or managed identity, a storage account key), in Unity
  Catalog's JSON shape. `Debug` redacts every secret.
- **`CredentialResolver`**: which registered credential governs a location.
  `ExternalLocations` picks the most specific registered location, on whole path
  segments, and refuses a location containing one that another credential governs.
- **`CredentialMinter`**: mints a `VendedCredential` covering exactly the requested
  scope at the requested access. `LocalMinter` serves `file://` and `memory://`.

The credential model and downscoping policies live in
[`olai-storage-credentials`](../olai-storage-credentials); the client side is
`olai-vended-store`.
