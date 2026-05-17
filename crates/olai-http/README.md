# olai-http

Unified cloud credential abstraction for AWS, Azure, GCP, and Databricks.

> [!IMPORTANT]
> The credential providers in this crate are extracted from the [object_store] crate's
> internal client and hoisted into a standalone crate for reuse by other projects.

## Motivation

Comprehensive SDKs exist for each major cloud provider, but when a service must
interact with multiple providers (or with third-party platforms like Databricks),
pulling in every vendor SDK brings dependency bloat and configuration fragmentation.

`olai-http` provides a single `CloudClient` that handles authentication for
AWS, Azure, GCP, and Databricks through a common `RequestSigner` trait, backed by
the same battle-tested credential machinery that powers `object_store`.

## Features

- **Unified `CloudClient`** — one HTTP client type for all providers
- **`RequestSigner` trait** — pluggable authentication (SigV4, bearer tokens, SAS, etc.)
- **Temporary credential vending** — STS AssumeRole (AWS), client-secret / workload-identity tokens (Azure), service-account tokens (GCP)
- **Azure SAS generation** — storage-key and user-delegation SAS for blob/container access
- **Databricks OAuth** — M2M, OIDC, and CLI-profile authentication flows
- **Recording mode** — capture HTTP interactions to JSON for test replay (`recording` feature)

## Quick start

```toml
[dependencies]
olai-http = "0.1"
```

```rust,ignore
use olai_http::CloudClient;

// Bearer token auth (works with any provider)
let client = CloudClient::new_with_token("my-token");
let resp = client.get("https://api.example.com/data").send().await?;

// AWS credentials from environment
let client = CloudClient::new_aws([("region", "us-east-1")], None)?;

// Azure credentials
let client = CloudClient::new_azure([("account_name", "myaccount")], None)?;
```

## Recording

Enable the `recording` feature to capture all HTTP interactions:

```toml
[dependencies]
olai-http = { version = "0.1", features = ["recording"] }
```

```rust,ignore
let mut client = CloudClient::new_unauthenticated();
client.set_recording_dir("/path/to/recordings".into())?;
// Interactions are saved as 0000.json, 0001.json, etc.
```

Sensitive headers (`Authorization`, `x-amz-security-token`, etc.) are automatically
redacted in recordings.

[object_store]: https://crates.io/crates/object_store
