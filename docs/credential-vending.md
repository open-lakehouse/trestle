# Credential vending and `object_store`

Status: **proposed**. This is a design record. The crates it describes do not exist
yet. Implementation is tracked in #185 and the phased issues listed under [Roadmap](#roadmap).

## Context

Many lakehouse protocols hand clients short-lived cloud storage credentials scoped to
a storage subpath, rather than routing every byte through the server:

- Unity Catalog REST (`temporary-{table,volume,path}-credentials`)
- the Unity Catalog Delta API (`storage-credentials` with `prefix` + `config`)
- Iceberg REST (`storage-credentials` on `loadTable`, and the `/credentials` endpoint)
- Delta / Open Sharing (temporary table and volume credentials, or presigned URLs)

All of them have the same flow:

```text
client ──(olai-http: authenticated request)──▶ server
                                              │ resolve location → registered principal
                                              │ mint downscoped credential (STS / SAS / GCS CAB)
client ◀──────── vended credential + expiry ──┘
   │
   ├─ map credential onto an object_store CredentialProvider
   ├─ re-vend before expiry (same securable, never wider)
   └─ route each path to the store whose credential covers it
      (credentials are prefix-scoped; DataFusion needs one store per scope)
```

The wire shape and the endpoint are the only parts specific to a protocol. Today the
rest is implemented several times:

| Concern | mangrove | delta-rs `object_store_unity` (fork) | `olai-http` |
|---|---|---|---|
| Credential model | `crates/credentials` `TemporaryCredential` (UC-free); four more shapes in proto, delta-api (×2), sharing | `TemporaryCredentials` plus Iceberg-style keys | — |
| Server minting | `credentials::vend_credential`, S3 session policy, unbounded process cache | — | `aws::assume_role_with_base`, `azure::sas` |
| Re-vend source | `SecurableRef` enum, hard-wired to the generated UC client | `CredentialVendor` trait | — |
| Refreshing provider | `UCCredentialProvider<T>` | `UnityCredentialProvider<T>` | `TokenCache` |
| Store builders | `to_store` **and** `store_from_vended_credential` | `store.rs` | — |
| Routing store | `datafusion/src/storage.rs` **and** `wasm/olai-delta-df` | `routing.rs` | — |
| Presign | `object-store/src/presign.rs`, sharing `FileSigner` | — | — |

Every copy has been fixed separately for the same bugs: null expiry, double-encoded
SAS, empty session tokens, R2 endpoints and access points. The copies have also drifted
apart, for example in their refresh margins (60 s vs 300 s).

## Decision

We will extract the pattern into three crates in this repository, next to
`olai-http`.

The client crate ships the vending protocols themselves, as feature-gated modules. This
follows the model of `object_store`, which ships a small client for each public
storage API (S3, Azure Blob, GCS), selected by URL and enabled by a feature. The
vending protocols are public specs in the same way. Each one needs one or two
endpoints, auth through `olai-http`, and its wire adapter from (1). With the protocols
included, `olai-vended-store` works out of the box against any of these vending
servers, much as `object_store` works against any of those clouds.

```text
                  olai-http   transport, retry, identity (bearer / cloud / Databricks)
                      │
        ┌─────────────┼─────────────────────────────────┐
        │             │                                 │
olai-storage-credentials (1)                            │
  model · locations · wire adapters · policy builders   │
        │                                               │
        ├──────────────────────┐                        │
        │                      │                        │
olai-vended-store (2)          olai-credential-vending (3) ──┘
  CLIENT                         SERVER
  core: source → provider →      principal → minter →
        store → router → presign downscoped credential
  protocols (features):
        unity · delta-api · iceberg · sharing · neon
        │
(4) downstream: vending *servers* (UC, sharing), catalog-managed commits,
    catalog browsing, generated catalog clients
```

### Design rules

- **Protocol-neutral core.** No core module in (1)–(3) names a catalog type. The only
  protocol-specific code is the `wire::*` adapters in (1) and the protocol modules in (2).
- **One-way dependencies.** In (2) the protocol modules depend on the core, and never
  the reverse. The core can then be split out mechanically later.
- **Hand-written protocol clients.** Protocol modules call their few endpoints
  directly, through `olai-http`, as `object_store` does instead of using the cloud
  SDKs. They never use the generated catalog clients (`olai-uc-client`, which is
  generated downstream in mangrove and depends on this repository). Using them would
  make the dependency graph circular and leak catalog models into a generic crate.
- **Narrow protocol scope.** A protocol module may contain:
  - its vend calls;
  - the minimum metadata lookup needed to turn a name into a storage location (for
    example GetTable);
  - its config keys;
  - its URL grammar.

  It may not contain catalog browsing, commit coordination or query APIs.
- **Donatable core.** The core of (1) and (2) depends on `object_store`, `serde` and
  `url`, and uses `olai-http` only behind a feature. The protocol features require
  `olai-http`. This keeps an `apache/object_store` donation of the core open (see
  [Upstream](#upstream)). (3) may depend on `olai-http` freely.
- **No widening on refresh.** A `CredentialSource` is bound to one securable and one
  access level when it is built, and a refresh replays exactly that request.
- **Segment-boundary prefixes.** `s3://b/tab` must not cover `s3://b/table2`. There is
  one implementation of this rule, in (1).
- **Secrets never print.** Every credential type redacts its `Debug` output.

### (1) `olai-storage-credentials`: model, locations, wire adapters

This crate builds on every target, including `wasm32`, and makes no HTTP calls.

```rust
pub enum AccessLevel { Read, ReadWrite }

pub enum StorageCredentials {               // Debug redacts
    Aws { key_id: String, secret_key: String, token: Option<String> }, // also R2
    AzureSas(Vec<(String, String)>),        // already percent-decoded
    AzureBearer(String),
    GcpBearer(String),
    None,                                   // file:// and other credential-less stores
}

pub struct VendedCredential {
    pub credentials: StorageCredentials,
    pub scope: StorageScope,                // the URL prefix the credential covers
    pub access: AccessLevel,
    pub expires_at: Option<SystemTime>,     // None = does not expire
    pub hints: StoreHints,                  // endpoint, region, access point, ...
}
```

- `StoreHints` carries the facts each store builder currently re-derives or hardcodes:
  the R2 or Azurite endpoint, the S3 region, the S3 access point, and path-style
  addressing.
- `location` holds the storage URL parsing and the single longest-prefix match. These
  are lifted from mangrove `crates/credentials/src/location.rs`.
- `policy` holds the pure downscoping documents: the S3 session policy (built with
  `serde_json` so a crafted prefix cannot break out), SAS permission sets, and GCS
  Credential Access Boundary rules. They are pure functions so they are unit-testable
  without a cloud.
- `wire::*` are feature-gated adapters, each a `TryFrom`/`From` pair:

| Adapter | Wire shape | Mapping notes |
|---|---|---|
| `wire::unity` | UC REST `TemporaryCredential` (`aws_temp_credentials`, `azure_user_delegation_sas`, `azure_aad`, `gcp_oauth_token`, `r2_temp_credentials`, `expiration_time` ms) | `expiration_time <= 0` means no expiry; SAS must be percent-decoded; `access_point` goes to `StoreHints` |
| `wire::iceberg` | Iceberg REST `storage-credentials[] { prefix, config }`, FileIO keys `s3.access-key-id`, `s3.secret-access-key`, `s3.session-token`, `s3.session-token-expires-at-ms`, `adls.sas-token.<account>`, `adls.sas-token-expires-at-ms.<account>`, `gcs.oauth2.token`, `gcs.oauth2.token-expires-at` | one `config` map may carry several credentials; `prefix` → `StorageScope` |
| `wire::delta_api` | UC Delta API `storage-credentials[] { prefix, operation, config, expiration-time-ms }`, keys `s3.*`, `azure.sas-token`, `gcs.oauth-token` | Iceberg-like keys with UC names; `null` expiry for local paths |
| `wire::sharing` | Open Sharing `SharingTemporaryCredentials` (AWS, Azure SAS, GCP OAuth, R2) | same fields as UC REST |

With these adapters, mangrove's five credential shapes reduce to one type plus four
mappings, and a new protocol only adds a mapping.

### (2) `olai-vended-store`: client side

This crate turns vended credentials into `object_store`s.

```rust
#[async_trait]
pub trait CredentialSource: Debug + Send + Sync + 'static {
    /// Vend a fresh credential for the securable this source is bound to.
    async fn fetch(&self) -> Result<VendedCredential>;
}

/// `object_store::CredentialProvider` for AwsCredential / AzureCredential / GcpCredential.
pub struct RefreshingCredentialProvider<T> { /* source, TokenCache<Arc<T>>, margin */ }

pub async fn build_store(
    seed: &VendedCredential,
    source: Arc<dyn CredentialSource>,
    options: &StoreOptions,
) -> Result<ScopedStore>;

pub struct ScopedStore { pub root: Arc<dyn ObjectStore>, pub prefix: Path, pub url: Url }
```

- `CredentialSource` generalizes delta-rs's `CredentialVendor`. A protocol implements
  it once per securable kind (UC tables, volumes and paths; Delta API tables; an
  Iceberg table; a shared table).
- `RefreshingCredentialProvider` behaves as follows:
  - It is seeded with the credential that was used to build the store, so the first
    request does not re-vend.
  - Refreshes are single-flight.
  - It re-vends `refresh_margin` before expiry. There is one default, configurable per
    store.
  - On `wasm32` it wraps the `!Send` fetch future (`SendWrapper`), so protocol code
    never has to.
- `build_store` is the single per-cloud builder: S3, R2, Azure, Azurite, GCS and
  local. It reads `StoreHints`, so nothing is hardcoded. It also handles S3 region
  discovery: configured region, then the access-point ARN, then an anonymous
  `HeadBucket` (cached), then the environment.
- `ScopedStore.root` is bucket-rooted and is what routing registers. A
  `PrefixStore`-wrapped view is available for callers that want relative paths.
- `RoutingObjectStore` sends each request to the store registered under the longest
  matching prefix. It refuses copies across routes, and returns `NotFound` on a miss.
- `VendingRouter` is optional. It is a `RoutingObjectStore` that fills misses lazily
  through a `SourceResolver`:

  ```rust
  #[async_trait]
  pub trait SourceResolver: Debug + Send + Sync + 'static {
      async fn source_for(&self, url: &Url, access: AccessLevel)
          -> Result<Option<Arc<dyn CredentialSource>>>;
  }
  ```

  This is the management layer DataFusion needs. A thin `ObjectStoreRegistry` adapter
  sits behind a `datafusion` feature.
- `presign` builds a store from a credential and calls `object_store::signer::Signer`
  for S3 and GCS. For Azure it appends the vended SAS. Sharing's `FileSigner` becomes
  a wrapper over it.

`wasm32`: the model and traits build as-is. The cloud store builders need what the
mangrove wasm workspace already configures: `getrandom`'s `js`/`wasm_js` backend and
the object_store wasm HTTP connector. Protocol modules use `olai-http-wasm` on
`wasm32`.

#### Protocol modules

Each protocol module implements `CredentialSource` for its securable kinds, and gets
its auth from a shared config layer.

| Feature | Vending server | Sources | Reference grammar |
|---|---|---|---|
| `unity` | Unity Catalog REST | table, volume, path (`temporary-*-credentials`; GetTable/GetVolume to resolve names) | `uc://catalog.schema.table`, `uc://catalog.schema.volume/path` (the delta-rs `object_store_unity` grammar) |
| `delta-api` | UC Delta API | table (`storage-credentials`; picks the most specific prefix) | via `unity` references |
| `iceberg` | Iceberg REST | table (`loadTable` `storage-credentials`, refresh via `/credentials`) | builder-only at first (see [Open questions](#open-questions)) |
| `sharing` | Delta / Open Sharing | volume, other storage-backed assets (`temporary-*-credentials`); auth from a sharing profile | `<profile>#share.schema.asset` (the delta-sharing client convention) |
| `neon` | Neon control plane | branch storage (endpoint discovery plus reveal or opt-in create of branch credentials) | Neon storage URL plus project and branch config |

The modules add two things to the core:

- **Dispatcher.** `parse_url_opts(url, options) -> (Arc<dyn ObjectStore>, Path)`
  recognizes each enabled protocol's reference grammar and builds the right source,
  in the style of `ObjectStoreScheme`. Storage-URL parsing (`s3://`, `abfss://`, …)
  stays in (1). Securable references are protocol grammar, so each module owns its
  own.
- **Config keys, in three tiers:**
  - generic auth keys in the core (static or file token, OAuth client credentials,
    token exchange, Azure and Google identity), built on `olai-http`;
  - per-protocol keys in each module (`unity_*`, `iceberg_*`, `sharing_*`, `neon_*`);
  - Databricks auto-detection (host, profiles, `DATABRICKS_*`) behind a separate
    `databricks` feature, so the core stays vendor-neutral.

Neon differs from the other protocols: its issuer is a control plane, not a catalog,
and it cannot downscope. Keys are long-lived and branch-scoped, with no STS. The
`neon` source therefore returns `StorageCredentials::Aws` with no expiry, plus
`StoreHints` (endpoint, region, path-style).

### (3) `olai-credential-vending`: server side

This crate is native only. It mints downscoped credentials from a registered principal.

```rust
pub struct MintRequest {
    pub location: Url,
    pub access: AccessLevel,
    pub ttl: Option<Duration>,
    pub session_name: Option<String>,  // audit: STS RoleSessionName, SAS signed identifier
}

#[async_trait]
pub trait CredentialMinter: Debug + Send + Sync + 'static {
    async fn mint(&self, request: &MintRequest) -> Result<VendedCredential>;
}

#[async_trait]
pub trait CredentialResolver: Send + Sync {
    /// The registered principal allowed to mint for `location`
    /// (longest-prefix match over the host's external locations).
    async fn resolve(&self, location: &StorageLocation) -> Result<RegisteredCredential>;
}
```

| Minter | Principal | Mechanism | Status |
|---|---|---|---|
| `AwsStsMinter` | IAM role (+ optional base keys, else ambient chain) | `AssumeRole` + inline session policy | exists (`olai-http::aws`); add duration, `ExternalId`, `SourceIdentity`, tags |
| `AzureUserDelegationSasMinter` | service principal / managed identity | user-delegation key → directory/container SAS | exists; cache the delegation key, fix the string-to-sign (below) |
| `AzureAccountKeySasMinter` | storage account key | service SAS | exists |
| `GcsDownscopedMinter` | service account / workload identity | STS token exchange with a Credential Access Boundary | **new** |
| `LocalMinter` | — | `StorageCredentials::None` for `file://` | exists |

- The principal (`RegisteredCredential`) supplies the identity used to call the cloud.
  It reuses `olai-http`'s identity flows (`BearerAuth`, the AWS chain, Azure
  SP/MSI/workload identity, GCP SA/WIF), so the server's own deployment identity and
  the registered principals use one mechanism.
- `CachingMinter` wraps any minter. It is a bounded TTL cache keyed by the structured
  request plus a principal fingerprint, and replaces mangrove's unbounded,
  `u64`-hashed process cache.
- Server-side presigning (Open Sharing `presigned` access) is minting followed by
  (2)'s `presign`.
- Stores without downscoping, Neon object storage for example, get no minter. A
  server backed by one presigns with its own key and never hands the key to clients.
- The low-level cloud calls (the STS query, SAS string-to-sign and delegation key)
  stay in `olai-http::{aws,azure,gcp}`, and the minters compose them. Reading
  `azure::sas::build_user_delegation_sas` shows two defects. Neither has been
  reproduced against Azure yet:
  - The string-to-sign has three empty slots before `https` where the spec has four
    (authorized OID, unauthorized OID, correlation ID, signed IP). As a result
    `https` lands in the signed-IP slot. Compare object_store's
    `string_to_sign_user_delegation_sas`.
  - It signs a non-empty `signedStart` but does not emit `st=`, so the service
    recomputes the signature with an empty start.

  Fix both, with a test against a known-good signature, before (3) depends on them.

### (4) Downstream

What stays outside this repository:

- **Vending servers.** These are the HTTP handlers that implement the UC, Iceberg or
  Sharing vending APIs. Each server owns its authorization and routing. It uses (3) to
  mint and the `wire::*` adapters in (1) to serialize.
- **Catalog behaviour beyond vending.** This covers catalog browsing, commit
  coordination for catalog-managed tables, and Delta Sharing query and file listing.
  Delta Sharing's presigned-file mode returns URLs rather than credentials, so it does
  not map onto a store.
- **Generated catalog clients**, such as `olai-uc-client`.

Adoption:

- **delta-rs** adopts rather than wraps. It depends on `olai-vended-store` with
  `unity`, and `object_store_unity` shrinks to a re-export or is dropped. The upstream
  PR becomes the `deltalake-catalog-unity` integration alone: `uc://` registration,
  the DataFusion schema provider, and catalog-managed commits.
- **mangrove**:
  - `olai-uc-credentials` becomes (1)+(3), re-exported or retired.
  - `olai-uc-object-store` reduces to the `unity` and `delta-api` features, plus
    whatever mangrove-only glue remains (forwarded-identity headers, storage-proxy
    access).
  - The DataFusion and wasm catalogs use `VendingRouter`.
  - The sharing server uses (3) for local access and the `unity` sources for delegated
    access.
  - The Neon factory planned for mangrove becomes the `neon` feature.

## Upstream

Candidates for `apache/object_store`, most likely first:

1. An `AssumeRole` credential provider with an optional session policy in
   `object_store::aws`. It only has web identity today.
2. A refreshing provider over a credential-source trait, plus `RoutingObjectStore`.
   These could go in the core crate or a sibling crate (cf. `object_store_opendal`).
   This is the core of (2) only. The protocol modules are catalog concepts and stay
   here.
3. GCS downscoped tokens (Credential Access Boundary).

`olai-http`'s forked client stays here regardless, because object_store's internal
client is not public. Proposals start only after (1) and (2) have shipped and been
adopted by mangrove and delta-rs.

## Consequences

- trestle gains a direct `object_store` dependency (0.13, matching mangrove and
  delta-rs). `olai-http`'s own dependencies are unchanged.
- The delta-rs `object_store_unity` stack (roeap/delta-rs#331–#335) waits for (1)+(2)
  with `unity` to be published, then adopts them. The new crate drops out of the
  upstream PR, so delta-rs takes on a dependency on an Open Lakehouse crate, as it
  already does for `olai-http`.
- One place to fix: future credential bugs (encoding, expiry, endpoints) land once.
- Adding a vending protocol means a feature, a wire adapter and a source, with no new
  crate.

## Open questions

- Crate names: `olai-storage-credentials`, `olai-vended-store`,
  `olai-credential-vending`.
- Whether `VendingRouter` and the DataFusion adapter live in (2), or in a fourth crate
  so (2) never sees DataFusion.
- Iceberg REST has no standard table-reference URL. Options are builder-only
  configuration, or a convention of ours such as
  `iceberg+https://host/prefix#namespace.table`.
- R2 minting through the Cloudflare temporary-credentials API, and S3-compatible
  stores that lack STS (handled like Neon: presign, don't mint).
- Hosting the vending server as a Neon Function, which needs no extra crate surface
  beyond (3).

## Roadmap

| Phase | Scope | Issue |
|---|---|---|
| P1 | `olai-storage-credentials`: model, locations, policy builders, wire adapters | #179 |
| P2 | `olai-vended-store` core: source, refreshing provider, builders, scoped/routing stores, presign, dispatcher, auth config | #180 |
| P2a | `unity` protocol module | #187 |
| P2b | `delta-api` protocol module | #188 |
| P2c | `iceberg` protocol module | #189 |
| P2d | `sharing` protocol module | #190 |
| P2e | `neon` protocol module | #191 |
| P3 | `olai-credential-vending`: minters, caching, resolver; needs the `olai-http` SAS fix (#178) | #181 |
| P4 | GCS downscoped minter | #182 |
| P5 | `VendingRouter` + DataFusion registry adapter | #183 |
| P6 | mangrove migration | open-lakehouse/mangrove#247 |
| P7 | delta-rs adopts `olai-vended-store` + `unity` | tracked in #185 (fork has no issues) |
| P8 | `apache/object_store` proposals | #184 |
