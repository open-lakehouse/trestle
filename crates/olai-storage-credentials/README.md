# olai-storage-credentials

The canonical model for vended cloud storage credentials: the short-lived, prefix-scoped
credentials that Unity Catalog, the UC Delta API, Iceberg REST and Open Sharing hand to
clients.

- **Model**: `VendedCredential` holds a `StorageCredentials` secret (AWS/R2 keys, Azure
  SAS, Azure or GCP bearer token), the `StorageScope` it covers, the `AccessLevel` it
  grants, its expiry and `StoreHints` (endpoint, region, access point, ...).
- **Locations**: every spelling of a storage URL (`s3://`, S3 `https://`, `abfss://`,
  Azure `https://`, Azurite, `gs://`, `r2://`, `file://`) parses to one canonical
  `StorageLocation`. Prefix matching is segment-aware: `s3://b/tab` covers
  `s3://b/tab/x`, never `s3://b/table2`.
- **SAS**: Azure SAS tokens are decoded exactly once, and their `se=` expiry is read.
- **Policies**: the S3 session policy, Azure SAS scoping and GCS Credential Access
  Boundary that downscope a minted credential to one scope.
- **Wire adapters** (features `unity`, `delta-api`, `iceberg`, `sharing`, or `wire` for
  all): read each vending protocol's response into the model, and write the model back
  out for servers.

The crate makes no network calls, never reads the clock, and builds for
`wasm32-unknown-unknown`. Secret-bearing types redact their `Debug` output.

```rust
use olai_storage_credentials::{AccessLevel, StorageCredentials, StorageLocation, VendedCredential};

let credential = VendedCredential::new(
    StorageCredentials::azure_sas("sv=2022-11-02&sp=rl&se=2030-01-01&sig=c2ln")?,
    "abfss://ctr@acct.dfs.core.windows.net/warehouse/tab".parse()?,
    AccessLevel::Read,
);
let file: StorageLocation = "https://acct.blob.core.windows.net/ctr/warehouse/tab/part-0.parquet".parse()?;
assert!(credential.covers(&file, AccessLevel::Read));
```
