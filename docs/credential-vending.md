# Credential vending and `object_store`

Status: **accepted, in progress**. This is a design record. The three crates exist:
P1 (`olai-storage-credentials`), P2 through P2e (`olai-vended-store` with the `unity`,
`delta-api`, `iceberg`, `sharing` and `neon` modules) and P3
(`olai-credential-vending`) have landed, with the `olai-http` SAS fix (#178). The sketches below follow the code; where the code
settled differently from the first draft, the text says so. Implementation is tracked
in #185 and the phased issues listed under [Roadmap](#roadmap).

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
- **Lean, donatable core.** (1) depends only on `object_store` (no default features),
  `serde`, `url` and `chrono`, and makes no HTTP calls. (2) owns its refresh cache, and
  depends on `olai-http` (native only) for region discovery and the protocol modules.
  `olai-http` is a fork of object_store's own client, so this does not block a donation;
  the two would be proposed together (see [Upstream](#upstream)). (3) depends on
  `olai-http` freely.
- **No widening on refresh.** A `CredentialSource` is bound to one securable and one
  access level when it is built, and a refresh replays exactly that request.
- **Segment-boundary prefixes.** `s3://b/tab` must not cover `s3://b/table2`. There is
  one implementation of this rule, in (1).
- **Secrets never print.** Every credential type redacts its `Debug` output.
- **No clock in (1).** `SystemTime::now()` panics on `wasm32-unknown-unknown`, so (1)
  stores expiry as `chrono::DateTime<Utc>` and every time-dependent helper takes `now`
  as an argument.
- **Vocabulary.** A *vended* credential (`StorageCredentials`, the short-lived secret a
  client receives) is not a *registered* credential (`RegisteredCredential` in (3), the
  long-lived principal a server mints from). Mangrove calls the latter
  `StorageCredential`; the new crates never do.

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
    pub expires_at: Option<DateTime<Utc>>,  // None = does not expire
    pub hints: StoreHints,                  // endpoint, region, access point, ...
}
```

- `StoreHints` carries the facts each store builder currently re-derives or hardcodes:
  the R2 or Azurite endpoint, the S3 region, the S3 access point, and path-style
  addressing.
- `location` holds the storage URL parsing and the single longest-prefix match. Every
  spelling of a location (`s3://`, S3 `https://` virtual-host and path-style, `abfss://`,
  `az://`, Azure `https://` blob and dfs, Azurite, `gs://`, `file://`, `memory://`) parses
  to one canonical `StorageLocation`, and matching only ever compares canonical forms,
  segment by segment. Mangrove's `location.rs` is the starting point but is not lifted
  as-is: its `abfss` container branch is unreachable (`host_str()` drops the
  `container@` userinfo) and its longest match compares raw URL lengths.
- `policy` holds the pure downscoping documents: the S3 session policy (built with
  `serde_json` so a crafted prefix cannot break out), SAS permission sets, and GCS
  Credential Access Boundary rules. They are pure functions so they are unit-testable
  without a cloud.
- `wire::*` are feature-gated adapters. A wire response rarely carries everything the
  model needs: UC REST may omit `url`, and none of the shapes say which access level was
  granted. So the client direction takes the caller's request as context,
  `to_vended(&VendContext { scope, access }) -> Result<Vec<VendedCredential>>`. A prefix
  in the response may narrow `scope` but never widen it: a broader prefix is clamped to
  `scope`, and a credential for unrelated storage is never used. Shapes that carry one
  credential (UC REST, Sharing) reject it; shapes that carry a list (Iceberg, Delta
  API) skip it, and fail when nothing relates. Of a list, `wire::select_covering`
  picks the most specific credential covering the request. The server direction is
  `From<&VendedCredential>`, or `TryFrom` where the shape cannot carry every credential
  kind (only UC REST carries Azure AD tokens). It writes config-map prefixes with a
  trailing `/`, because clients of those protocols often match prefixes as plain
  strings.
- Expiry precedence, for every adapter: an explicit wire expiry wins; else the SAS `se=`
  parameter; else `None`. There is no silent default lifetime.

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
pub struct RefreshingCredentialProvider<T> { /* source, cached credential, margin, clock */ }

pub async fn build_store(
    seed: &VendedCredential,
    source: Arc<dyn CredentialSource>,
    options: &StoreOptions,
) -> Result<ScopedStore>;

pub struct ScopedStore { /* root, prefix, scope */ }
impl ScopedStore {
    pub fn root(&self) -> Arc<dyn ObjectStore>;    // bucket-rooted
    pub fn prefix(&self) -> &Path;                 // what the credential covers
    pub fn scope(&self) -> &StorageScope;
    pub fn url(&self) -> Url;
    pub fn prefixed(&self) -> Arc<dyn ObjectStore>; // paths relative to `prefix`
    pub fn child(&self, path: &str) -> Result<ScopedStore>; // e.g. a volume sub-path
}
```

- `CredentialSource` generalizes delta-rs's `CredentialVendor`. A protocol implements
  it once per securable kind (UC tables, volumes and paths; Delta API tables; an
  Iceberg table; a shared table).
- `RefreshingCredentialProvider` behaves as follows:
  - It is seeded with the credential that was used to build the store, so the first
    request does not re-vend.
  - Refreshes are single-flight. A failed refresh is not cached.
  - It re-vends `refresh_margin` before expiry (5 minutes by default, configurable per
    store), or halfway through a lifetime shorter than twice the margin, so short
    grants are not re-vended on every request.
  - A refresh that no longer covers the seed's scope and access level fails rather
    than serving a narrower credential.
  - It owns its cache rather than reusing `olai-http`'s `TokenCache`. That cache keys
    expiry on `std::time::Instant` and holds a tokio mutex, neither of which works on
    `wasm32`, and it can be neither seeded nor given a margin. This cache compares the
    credential's own `expires_at` against an injectable clock, so it builds everywhere
    and is testable without waiting.
  - On `wasm32` it wraps the `!Send` fetch future (`SendWrapper`), so protocol code
    never has to. `CredentialSource` itself is declared
    `#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]`, because `olai-http-wasm`
    futures are `!Send` and could not implement a `Send`-bound trait.
- `build_store` is the single per-cloud builder: S3, R2, Azure, Azurite, GCS,
  `file://` and `memory://`. It reads `StoreHints`, so nothing is hardcoded; a
  credential of `StorageCredentials::None` builds an unsigned store. `StoreOptions`
  carries the refresh margin and clock, client options, an I/O runtime and an S3
  endpoint for servers backed by S3-compatible storage, whose responses name none.
- S3 region discovery, in order: the configured region, the vended hint, the
  access-point ARN, an anonymous `HeadBucket` (`olai-http`'s `aws::bucket_region`,
  never signed and never retried; hits cached for a day, failures not cached; AWS
  only), then the environment.
- Azure: a loopback endpoint hint is Azurite, addressed path-style. object_store's
  emulator mode is not used, because it ignores credential providers and the store
  could not refresh.
- `ScopedStore::root` is bucket-rooted and is what routing registers. The credential
  enforces the prefix and access; stores that need none (`file://`, `memory://`)
  enforce neither, and are a convenience, not a security boundary.
- `RoutingObjectStore` sends each request to the store registered under the longest
  matching prefix, on whole segments. All its routes must be in one bucket, since
  paths carry none. It refuses copies and renames across routes, needs a prefix to
  list, and returns `NotFound` on a miss.
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
- `presign` refuses any location the credential does not cover at the access the
  method needs, and never outlives the credential:
  - S3 and R2: SigV4 query-signing with the vended keys, for `min(ttl, expiry)`.
  - Azure with a SAS: the SAS appended to the blob URL. A SAS cannot be shortened, so
    the URLs live as long as the credential whatever `ttl` asks for, and the returned
    expiry says so.
  - Azure with an AD token: a user-delegation SAS signed for the requested lifetime.
  - GCS cannot be presigned from a vended OAuth token (signing needs a
    service-account key), so it is refused.

  Sharing's `FileSigner` becomes a wrapper over it.

`wasm32`: the core and all three store backends build for `wasm32-unknown-unknown`.
The crate selects, behind target-gated dependencies, the browser RNG backends
object_store's cloud clients need there (getrandom 0.2 `js`, getrandom 0.4 `wasm_js`,
ring `wasm32_unknown_unknown_js`). Region discovery and the protocol modules use
`olai-http` and are native only for now; see the `wasm32` build of `olai-http` under
[Open questions](#open-questions).

#### Protocol modules

Each protocol module implements `CredentialSource` for its securable kinds, and gets
its auth from a shared config layer.

| Feature | Vending server | Sources | Reference grammar |
|---|---|---|---|
| `unity` (**done**) | Unity Catalog REST | table, volume, path (`temporary-*-credentials`; one GetTable/GetVolume lookup resolves a name to its id and location) | `uc:///Tables/c/s/t`, `uc:///Volumes/c/s/v[/path]`, `vol+dbfs:/Volumes/...`, the legacy `uc://c.s.t` (the delta-rs `object_store_unity` grammar), and storage URLs for path credentials |
| `delta-api` (**done**) | UC Delta API | table (`storage-credentials`; the most specific prefix covering the table, then clamped to it); selected with `unity_delta_api` | via `unity` references |
| `iceberg` (**done**) | Iceberg REST | table (`loadTable` with `X-Iceberg-Access-Delegation`, `storage-credentials` or legacy config, refresh via `/credentials` with a `loadTable` fallback; one store per vended prefix on request) | builder-only (see [Open questions](#open-questions)) |
| `sharing` (**done**) | Delta / Open Sharing | volume, agent skill (`temporary-*-credentials`); endpoint and token from a sharing profile | `share.schema.volume[/path]` (the part after a delta-sharing profile's `#`) or `sharing:///share/schema/volume[/path]`; the profile is an option (`sharing_profile`), never part of the URL |
| `neon` (**done**) | Neon control plane | branch storage (endpoint discovery plus reveal or opt-in create of branch credentials) | builder-only: `s3://bucket[/prefix]` plus project and branch config |

The modules add two things to the core:

- **Dispatcher.** `parse_url_opts(url, options) -> (Arc<dyn ObjectStore>, Path)`
  recognizes each enabled protocol's reference grammar (`Protocol::of`), vends, and
  returns the bucket-rooted store and the path the reference names. It is `async`,
  unlike object_store's, because it vends. Storage URLs (`s3://`, `abfss://`, …) belong
  to no protocol and are left to object_store. Securable references are protocol
  grammar, so each module owns its own.
- **Config keys, in three tiers:**
  - generic auth keys in the core `auth` module, shared by every protocol and
    differing only in their prefix (`unity_token`, `iceberg_token`, …): static, file
    and environment tokens, OAuth client credentials, token exchange, Azure and Google
    identity, built on `olai-http`. Cloud identities are only used once scoped to the
    server (an OAuth scope or audience);
  - per-protocol keys in each module (`unity_catalog_url`, `unity_operation`,
    `unity_delta_api`, `aws_endpoint`, …; later `iceberg_*`, `sharing_*`, `neon_*`);
  - Databricks auto-detection (host, profiles, `DATABRICKS_*`) behind a separate
    `databricks` feature, so the core stays vendor-neutral. Only `databricks_`-prefixed
    keys are read, so a shared options map's `token` or `host` is never captured.
- **Read fallback.** With `unity_operation` unset, `UnityCatalog::open` asks for
  read-write and falls back to read only when the vend is denied (HTTP 403 or
  `PERMISSION_DENIED`); any other failure is returned. Iceberg REST cannot request an
  access level and Sharing is read-only, so their stores are declared read (Iceberg:
  `iceberg_operation` to declare read-write).
- **Shared client plumbing.** The modules share one JSON client (`api`), whose
  `ApiError` carries the status and the protocol's error code, so callers tell a
  denial from an outage the same way for every protocol.

Neon differs from the other protocols: its issuer is a control plane, not a catalog,
and it cannot downscope. It is operator tooling for the owner of the branch, not a
vending protocol in the sense above: a vending *server* must never hand these keys to
clients (see (3)). Keys are long-lived and branch-scoped, with no STS. The
`neon` source therefore returns `StorageCredentials::Aws` with no expiry, plus
`StoreHints` (endpoint, region, path-style).

### (3) `olai-credential-vending`: server side

This crate is native only. It mints downscoped credentials from a registered principal.

```rust
pub struct MintRequest {
    pub scope: StorageScope,           // canonical, so no minter re-parses a URL
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
| `AwsStsMinter` | IAM role (+ optional base keys, else ambient chain) | `AssumeRole` + inline session policy; duration, session name, `ExternalId`, `SourceIdentity`, tags | **done** |
| `AzureUserDelegationSasMinter` | service principal / managed identity | user-delegation key (cached across mints) → directory/container SAS | **done** |
| `AzureAccountKeySasMinter` | storage account key | service SAS | **done** |
| `GcsDownscopedMinter` | service account / workload identity | STS token exchange with a Credential Access Boundary | P4 |
| `LocalMinter` | — | `StorageCredentials::None` for `file://` and `memory://` | **done** (called directly: no registered credential governs local storage, so `minter_for` and `Vendor` never return it) |

`minter_for(&RegisteredCredential)` picks the minter for each registered kind. On
flat-namespace accounts (`with_flat_namespace`, implied for Azurite) the Azure minters
mint for whole containers only: a container SAS for a directory request would widen
the grant. User delegation keys are cached per storage account.

- The principal (`RegisteredCredential`) supplies the identity used to call the cloud.
  It reuses `olai-http`'s identity flows (`BearerAuth`, the AWS chain, Azure
  SP/MSI/workload identity, GCP SA/WIF), so the server's own deployment identity and
  the registered principals use one mechanism.
- `RegisteredCredential` keeps Unity Catalog's `Credential` JSON shape, so documents
  mangrove persisted as `StorageCredential` load unchanged; unlike mangrove's, its
  `Debug` redacts every secret. `ExternalLocations` is the longest-prefix
  `CredentialResolver` over a list.
- `CachingMinter` wraps any minter. It is a bounded TTL cache keyed by the whole
  request (canonical scope, access, lifetime, session name), and replaces mangrove's
  unbounded, `u64`-hashed process cache. One cache serves one principal: `Vendor`
  resolves the governing credential and mints through a `CachingMinter` per
  registered credential, which is the whole server side of a
  `temporary-*-credentials` endpoint once the caller is authorized.
- Server-side presigning (Open Sharing `presigned` access) is minting followed by
  (2)'s `presign`.
- Stores without downscoping, Neon object storage for example, get no minter. A
  server backed by one presigns with its own key and never hands the key to clients.
- The low-level cloud calls (the STS query, SAS string-to-sign and delegation key)
  stay in `olai-http::{aws,azure,gcp}`, and the minters compose them:
  `aws::assume_role_with_options` (every `AssumeRole` parameter, and the wall-clock
  expiry), and `azure::fetch_user_delegation_key` with `build_user_delegation_sas` /
  `build_storage_key_sas`, so a minter can fetch one key and sign many SAS.
- The user-delegation SAS string-to-sign defects found while writing this record
  (misaligned slots, `st=` signed but not emitted, `sdd` signed) were fixed in #178,
  with known-good signature vectors and Azurite tests.

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

(2) depends on `olai-http`, a fork of object_store's internal client (which is not
public). A donation of (2) would therefore go together with `olai-http`, or with
object_store exposing its client. We would work with the community on that rather than
contort (2) to avoid the dependency. Proposals start only after (1) and (2) have shipped
and been adopted by mangrove and delta-rs.

## Testing

Downscoping and signing are security properties, so each claim is tested against a
service that enforces it wherever an emulator can. Golden-output unit tests complement
these; they do not replace them.

| Cloud | Emulator | Verifies | Cannot verify |
|---|---|---|---|
| S3 + STS | RustFS (`rustfs/rustfs`) | `AssumeRole` with an inline session policy, enforced: allowed inside the prefix, denied outside it and on sibling prefixes (`tab` vs `table2`) | AWS-only request keys (`ExternalId`, `SourceIdentity`, tags) |
| Azure | Azurite, with `--oauth basic` for user-delegation keys | service and user-delegation SAS signatures | directory SAS on HNS (`sr=d`); Azurite has no ADLS Gen2 |
| GCS | fake-gcs-server (not yet wired; a mock endpoint checks bearer signing meanwhile) | bearer-token store I/O | Credential Access Boundaries; there is no STS emulator |

What no emulator covers runs as opt-in jobs against real cloud accounts. MinIO
(community edition archived in 2026) and LocalStack (IAM enforcement is a paid feature)
are not used.

Protocol modules are tested against recorded server responses (mockito), shared with
the wire adapter tests where they exist. The sharing module is not tested against
mangrove's `olai-sharing-server` here, because mangrove depends on this repository;
that run belongs to mangrove's adoption (P6).

An end-to-end suite in (3) runs the whole path: a `Vendor` mints, the credential is
serialized as a UC REST response, and the client parses it and builds a store with
(2)'s `build_store`; the emulators then enforce the grant, across refreshes.

Containers are managed with `testcontainers` from a shared, unpublished
`olai-test-emulators` crate. Emulator tests sit behind an `emulator-it` feature, so the
default `cargo test` needs no Docker, and run in a dedicated CI job.

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

- Whether `VendingRouter` and the DataFusion adapter live in (2), or in a fourth crate
  so (2) never sees DataFusion.
- Iceberg REST has no standard table-reference URL. The `iceberg` module is
  builder-only for now; a convention of ours, such as
  `iceberg+https://host/prefix#namespace.table`, would let `parse_url_opts` dispatch
  to it.
- Open Sharing profiles name the server the API paths are relative to, but
  mangrove's sharing server writes its Delta Sharing mount (`/api/v1/delta-sharing`)
  into profiles while serving credentials under `/api/1.0/opensharing`. Until its
  profiles name the Open Sharing endpoint, clients set `sharing_endpoint`.
- R2 minting through the Cloudflare temporary-credentials API, and S3-compatible
  stores that lack STS (handled like Neon: presign, don't mint).
- Hosting the vending server as a Neon Function, which needs no extra crate surface
  beyond (3).
- A `wasm32` build of `olai-http`. Protocol modules are native only today; on
  `wasm32` they would need `olai-http-wasm`, a second client. Target-gating `olai-http` would let one client serve
  both and fold `olai-http-wasm` in.

## Roadmap

| Phase | Scope | Issue |
|---|---|---|
| — | `olai-http` user-delegation SAS fix (**done**, #197) | #178 |
| P1 | `olai-storage-credentials`: model, locations, policy builders, wire adapters (**done**, #192 #193 #194) | #179 |
| P2 | `olai-vended-store` core: source, refreshing provider, `build_store`, `ScopedStore` (**done**, #200 #201) | #180 |
| P2.1 | routing store and presign (**done**, #203) | #180 |
| P2a | `unity` protocol module, with the dispatcher and the auth config tiers (**done**, #204 #205) | #187 |
| P2b | `delta-api` protocol module (**done**, #206) | #188 |
| P2c | `iceberg` protocol module (**done**) | #189 |
| P2d | `sharing` protocol module (**done**) | #190 |
| P2e | `neon` protocol module (**done**) | #191 |
| P3 | `olai-credential-vending`: minters, caching, resolver (**done**, #207 #208 #210 #211 #212 #213) | #181 |
| P4 | GCS downscoped minter | #182 |
| P5 | `VendingRouter` + DataFusion registry adapter | #183 |
| P6 | mangrove migration | open-lakehouse/mangrove#247 |
| P7 | delta-rs adopts `olai-vended-store` + `unity` | tracked in #185 (fork has no issues) |
| P8 | `apache/object_store` proposals | #184 |

The delta-rs critical path (P1 → P2 → P2a) has landed; P7 waits only on the release
below. All protocol modules (P2a–P2e) have landed. Next: P4 (GCS) and P5
(`VendingRouter`).

The new crates are not published yet: they stay out of the release cycle
(`release = false`) so their APIs can still change. Their in-repo dependencies name
both a `path` and a `version`, which release-plz keeps in sync. They are released one
crate at a time in dependency order: `olai-storage-credentials`, then
`olai-vended-store`, then `olai-credential-vending`.
