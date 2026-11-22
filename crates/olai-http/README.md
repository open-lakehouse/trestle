# olai-http

Unified cloud credential abstraction and HTTP client for AWS, Azure, GCP, and
Databricks.

> [!NOTE]
> The credential providers are extracted from the [`object_store`] crate's
> internal client.

When a service talks to multiple clouds (or to platforms like Databricks),
pulling in every vendor SDK means dependency bloat and config fragmentation.
`olai-http` offers a single `CloudClient` that authenticates against all of them
through one `RequestSigner` trait, backed by the same credential machinery that
powers `object_store`.

## Features

- **Unified `CloudClient`** — one HTTP client type for every provider
- **`RequestSigner` trait** — pluggable auth (SigV4, bearer, SAS, …)
- **Temporary credentials** — STS AssumeRole (AWS), client-secret / workload-identity (Azure), service-account tokens (GCP)
- **Azure SAS** — storage-key and user-delegation SAS for blob/container access
- **Databricks OAuth** — M2M, OIDC, and CLI-profile flows
- **Bearer tokens for any API** (`bearer` module) — OAuth client credentials against any token endpoint, Azure AD (service principal, managed identity, workload identity, CLI) and GCP ID tokens for a configurable scope/audience, tokens from env vars or files, and RFC 8693 token exchange
- **Secrets over SDS** — a named secret fetched from a secret server over a Unix socket and watched for rotation (`sds` feature)
- **Recording mode** — capture HTTP interactions to JSON for test replay (`recording` feature)

## Usage

```toml
[dependencies]
olai-http = "0.0"
```

```rust,ignore
use olai_http::CloudClient;

// Bearer token (any provider)
let client = CloudClient::new_with_token("my-token");
let resp = client.get("https://api.example.com/data").send().await?;

// A token for your own API, from an Azure managed identity, exchanged for the
// API's access token (RFC 8693)
use olai_http::bearer::{AzureManagedIdentity, BearerAuth, TokenExchange};
let auth = BearerAuth::new();
let id_token = auth.azure_managed_identity(AzureManagedIdentity {
    resource: "api://my-api".into(),
    ..Default::default()
})?;
let provider = auth.token_exchange(TokenExchange {
    endpoint: "https://api.example.com/oauth/token".into(),
    subject: id_token,
    subject_token_type: None,
    requested_token_type: None,
    audience: None,
    scope: None,
})?;
let client = CloudClient::new_with_bearer(provider, &Default::default(), Default::default(), None)?;

// Provider credentials from a key/value config
let client = CloudClient::new_aws([("region", "us-east-1")], None)?;
let client = CloudClient::new_azure([("account_name", "myaccount")], None)?;
```

### Secrets over SDS

With the `sds` feature, `SdsCredentialProvider` reads one named secret from a
secret server (such as `olai-secret-server`) that speaks Envoy's v3 secret
discovery service over a Unix socket. The socket is the consumer's identity:
the server only answers it the names it may see, and any other name fails
with `Error::NotFound`.

```rust,ignore
use olai_http::CredentialProvider;
use olai_http::bearer::BearerAuth;
use olai_http::sds::SdsCredentialProvider;

// Any secret, as bytes: a key-encryption key, say.
let kek = SdsCredentialProvider::new("/run/trestle-sds/sds.sock", "server-kek");
let key = kek.get_credential().await?;
let bytes: &[u8] = key.expose_bytes();

// A bearer token for `CloudClient::new_with_bearer`.
let token = BearerAuth::new().sds("/run/trestle-sds/sds.sock", "upstream-token");
```

- The first `get_credential` fetches the secret (`FetchSecrets`). While the
  socket is unreachable it retries for up to 30 seconds (`with_connect_timeout`).
- It then keeps a `StreamSecrets` watch open in the background. It
  acknowledges each version with its nonce and swaps the cached value when the
  server sends a new one, so a rotation reaches the service without a restart.
- When the server goes away, for example when it's recreated to rotate a
  value, the provider keeps the last value and reconnects with backoff.
- Later calls return the cached value without I/O.
- `SdsSecret` redacts its value in `Debug` and has no `Display`.

The feature pulls in tonic and the Envoy protos (`envoy-types`). The default
build doesn't.

### Recording

With the `recording` feature, captured interactions are written as
`0000.json`, `0001.json`, … with sensitive headers (`Authorization`,
`x-amz-security-token`, …) automatically redacted:

```rust,ignore
let mut client = CloudClient::new_unauthenticated();
client.set_recording_dir("/path/to/recordings".into())?;
```

## License

Apache-2.0

[`object_store`]: https://crates.io/crates/object_store
