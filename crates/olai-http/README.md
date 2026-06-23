# olai-http

Unified cloud credential abstraction and HTTP client for AWS, Azure, GCP, and
Databricks.

> [!NOTE]
> The credential providers are extracted from the [`object_store`] crate's
> internal client and hoisted into a standalone crate for reuse.

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

// Provider credentials from a key/value config
let client = CloudClient::new_aws([("region", "us-east-1")], None)?;
let client = CloudClient::new_azure([("account_name", "myaccount")], None)?;
```

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
