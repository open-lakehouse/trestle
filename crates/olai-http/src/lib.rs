//! Unified cloud credential abstraction and HTTP client for AWS, Azure, GCP, and
//! Databricks.
//!
//! This crate provides a single authenticated HTTP client, [`CloudClient`], that
//! signs outgoing requests for any supported cloud provider. Rather than pulling
//! in a separate vendor SDK for each cloud, a service constructs one
//! [`CloudClient`] per provider and issues requests through a familiar,
//! `reqwest`-style builder ([`CloudRequestBuilder`]). Every provider is reached
//! through the same [`RequestSigner`] trait, so credential resolution, token
//! refresh, and request signing are uniform across clouds. The credential
//! machinery is extracted from the
//! [`object_store`](https://crates.io/crates/object_store) crate's internal client.
//!
//! # Providers
//!
//! Each provider has a builder under its own module and a matching
//! [`CloudClient`] constructor:
//!
//! - **AWS** ([`aws`]) — SigV4 signing with static keys, IMDS, ECS/EKS task
//!   roles, web identity, and STS `AssumeRole`. See [`CloudClient::new_aws`].
//! - **Azure** ([`azure`]) — Azure AD bearer tokens via client secret, managed
//!   identity, workload identity, or the Azure CLI. See [`CloudClient::new_azure`].
//! - **Google Cloud** ([`gcp`]) — OAuth 2.0 bearer tokens via service-account
//!   JWTs, the GCE metadata server, or workload identity federation. See
//!   [`CloudClient::new_google`].
//! - **Databricks** ([`databricks`]) — OAuth M2M and OIDC token exchange. See
//!   [`CloudClient::new_databricks`].
//!
//! For a static token or no authentication at all, use
//! [`CloudClient::new_with_token`] or [`CloudClient::new_unauthenticated`].
//!
//! # Examples
//!
//! ```no_run
//! use olai_http::CloudClient;
//!
//! # async fn run() -> olai_http::Result<()> {
//! let client = CloudClient::new_with_token("my-token");
//! let resp = client
//!     .get("https://api.example.com/data")
//!     .send()
//!     .await?;
//! println!("status: {}", resp.status());
//! # Ok(())
//! # }
//! ```
//!
//! Enable the `recording` feature to capture HTTP interactions to JSON (with
//! sensitive headers redacted) for test replay.

#[cfg(feature = "recording")]
use std::collections::HashMap;
#[cfg(feature = "recording")]
use std::path::PathBuf;
use std::sync::Arc;
#[cfg(feature = "recording")]
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::{Body, Client, IntoUrl, Method, RequestBuilder};
use serde::Serialize;
use tokio::runtime::Handle;

use self::retry::RetryExt;
use self::service::{HttpService, make_service};
pub use self::token::{TemporaryToken, TokenCache};

pub mod aws;
pub mod azure;
mod backoff;
mod client;
mod config;
#[cfg(feature = "connectrpc")]
pub mod connectrpc;
mod credential;
pub mod databricks;
mod error;
pub mod gcp;
pub mod service;

mod retry;
mod token;
mod util;

pub use client::{Certificate, ClientConfigKey, ClientOptions};
pub use credential::*;
pub use error::*;
pub use retry::RetryConfig;
pub use service::{ReqwestService, SpawnService};

/// A shared, cloneable signer for a `CloudClient`.
type SharedSigner = Arc<dyn RequestSigner>;

/// A no-op signer used by `new_unauthenticated`.
#[derive(Debug)]
struct NoopSigner;

impl RequestSigner for NoopSigner {
    fn sign<'a>(
        &'a self,
        req: RequestBuilder,
    ) -> futures::future::BoxFuture<'a, Result<RequestBuilder>> {
        Box::pin(async move { Ok(req) })
    }
}

/// A signer that injects a static bearer token.
#[derive(Debug)]
struct BearerTokenSigner {
    token: String,
}

impl RequestSigner for BearerTokenSigner {
    fn sign<'a>(
        &'a self,
        req: RequestBuilder,
    ) -> futures::future::BoxFuture<'a, Result<RequestBuilder>> {
        let token = self.token.clone();
        Box::pin(async move { Ok(req.bearer_auth(&token)) })
    }
}

#[cfg(feature = "recording")]
#[derive(Debug, Clone)]
struct RecordingState {
    out_dir: PathBuf,
    counter: Arc<AtomicU64>,
}

/// An authenticated HTTP client for cloud provider APIs.
///
/// Created via the provider-specific constructors [`CloudClient::new_aws`],
/// [`CloudClient::new_azure`], [`CloudClient::new_google`], or
/// [`CloudClient::new_databricks`], or the simpler [`CloudClient::new_with_token`]
/// and [`CloudClient::new_unauthenticated`].
#[derive(Clone)]
pub struct CloudClient {
    signer: SharedSigner,
    reqwest_client: Client,
    service: Arc<dyn HttpService>,
    /// Retry configuration applied to requests sent through this client.
    ///
    /// Used both by credential providers (token refresh) and by user-initiated
    /// requests via [`CloudRequestBuilder::send`] and
    /// [`CloudClient::sign_and_send`]. Override with
    /// [`CloudClient::with_retry_config`].
    pub retry_config: RetryConfig,
    #[cfg(feature = "recording")]
    recording: Option<RecordingState>,
}

impl CloudClient {
    fn new_with_signer(
        signer: SharedSigner,
        reqwest_client: Client,
        service: Arc<dyn HttpService>,
        retry_config: RetryConfig,
    ) -> Self {
        Self {
            signer,
            reqwest_client,
            service,
            retry_config,
            #[cfg(feature = "recording")]
            recording: None,
        }
    }

    /// Create a new client with AWS credentials.
    ///
    /// If `runtime` is provided, all HTTP I/O (including credential refresh)
    /// will be spawned on the given runtime handle.
    pub fn new_aws<I, K, V>(options: I, runtime: Option<&Handle>) -> Result<Self>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: Into<String>,
    {
        let config = options
            .into_iter()
            .fold(
                aws::AmazonBuilder::new(),
                |builder, (key, value)| match key.as_ref().parse() {
                    Ok(k) => builder.with_config(k, value),
                    Err(_) => builder,
                },
            )
            .build(runtime)?;

        let reqwest_client = config.client_options.client()?;
        let service = make_service(reqwest_client.clone(), runtime);
        let retry_config = config.retry_config.clone();
        Ok(Self::new_with_signer(
            Arc::new(config),
            reqwest_client,
            service,
            retry_config,
        ))
    }

    /// Create a new client with Google Cloud credentials.
    ///
    /// If `runtime` is provided, all HTTP I/O (including credential refresh)
    /// will be spawned on the given runtime handle.
    pub fn new_google<I, K, V>(options: I, runtime: Option<&Handle>) -> Result<Self>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: Into<String>,
    {
        let config = options
            .into_iter()
            .fold(
                gcp::GoogleBuilder::new(),
                |builder, (key, value)| match key.as_ref().parse() {
                    Ok(k) => builder.with_config(k, value),
                    Err(_) => builder,
                },
            )
            .build(runtime)?;

        let reqwest_client = config.client_options.client()?;
        let service = make_service(reqwest_client.clone(), runtime);
        let retry_config = config.retry_config.clone();
        Ok(Self::new_with_signer(
            Arc::new(config),
            reqwest_client,
            service,
            retry_config,
        ))
    }

    /// Create a new client with Azure credentials.
    ///
    /// If `runtime` is provided, all HTTP I/O (including credential refresh)
    /// will be spawned on the given runtime handle.
    pub fn new_azure<I, K, V>(options: I, runtime: Option<&Handle>) -> Result<Self>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: Into<String>,
    {
        let config = options
            .into_iter()
            .fold(
                azure::AzureBuilder::new(),
                |builder, (key, value)| match key.as_ref().parse() {
                    Ok(k) => builder.with_config(k, value),
                    Err(_) => builder,
                },
            )
            .build(runtime)?;

        let reqwest_client = config.client_options.client()?;
        let service = make_service(reqwest_client.clone(), runtime);
        let retry_config = config.retry_config.clone();
        Ok(Self::new_with_signer(
            Arc::new(config),
            reqwest_client,
            service,
            retry_config,
        ))
    }

    /// Create a new client with Databricks credentials.
    ///
    /// If `runtime` is provided, all HTTP I/O (including credential refresh)
    /// will be spawned on the given runtime handle.
    pub fn new_databricks<I, K, V>(options: I, runtime: Option<&Handle>) -> Result<Self>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: Into<String>,
    {
        use databricks::DatabricksBuilder;

        let config = options
            .into_iter()
            .fold(
                DatabricksBuilder::new(),
                |builder, (key, value)| match key.as_ref().parse() {
                    Ok(k) => builder.with_config(k, value),
                    Err(_) => builder,
                },
            )
            .build(runtime)?;

        let reqwest_client = config.client_options.client()?;
        let service = make_service(reqwest_client.clone(), runtime);
        let retry_config = config.retry_config.clone();
        Ok(Self::new_with_signer(
            Arc::new(config),
            reqwest_client,
            service,
            retry_config,
        ))
    }

    /// Create a new client with a personal access token.
    pub fn new_with_token(token: impl ToString) -> Self {
        let reqwest_client = Client::new();
        let service: Arc<dyn HttpService> = Arc::new(ReqwestService::new(reqwest_client.clone()));
        Self::new_with_signer(
            Arc::new(BearerTokenSigner {
                token: token.to_string(),
            }),
            reqwest_client,
            service,
            RetryConfig::default(),
        )
    }

    /// Create a new unauthenticated client.
    pub fn new_unauthenticated() -> Self {
        let reqwest_client = Client::new();
        let service: Arc<dyn HttpService> = Arc::new(ReqwestService::new(reqwest_client.clone()));
        Self::new_with_signer(
            Arc::new(NoopSigner),
            reqwest_client,
            service,
            RetryConfig::default(),
        )
    }

    /// Route all HTTP I/O through the given runtime handle.
    ///
    /// This is useful for simple constructors (`new_with_token`, `new_unauthenticated`)
    /// where no credential providers need to perform HTTP I/O. For cloud provider
    /// constructors, pass the handle at construction time instead.
    pub fn with_runtime(mut self, handle: Handle) -> Self {
        self.service = Arc::new(SpawnService::new(self.service, handle));
        self
    }

    /// Replace the [`HttpService`] used for request execution.
    pub fn with_http_service(mut self, service: Arc<dyn HttpService>) -> Self {
        self.service = service;
        self
    }

    /// Override the [`RetryConfig`] applied to requests sent through this client.
    ///
    /// Requests issued via [`CloudRequestBuilder::send`] and
    /// [`sign_and_send`](Self::sign_and_send) are retried per this config
    /// (exponential backoff with jitter; safe/idempotent requests are also
    /// retried on timeout). The [default](RetryConfig::default) allows up to 10
    /// retries — lower it for latency-sensitive interactive calls.
    pub fn with_retry_config(mut self, retry_config: RetryConfig) -> Self {
        self.retry_config = retry_config;
        self
    }

    /// Sign an already-built [`reqwest::Request`] and dispatch it, with retries.
    ///
    /// This is the request-level counterpart to the [`CloudRequestBuilder`]
    /// flow: it applies the client's [`RequestSigner`] (refreshing credentials
    /// as needed) and sends the request through the configured [`HttpService`],
    /// retrying transient failures per the client's [`RetryConfig`]. It is
    /// useful when the request is produced elsewhere (for example by a protocol
    /// stack such as ConnectRPC) and only needs authentication and transport.
    ///
    /// The request body must be in memory (not a stream): retries clone the
    /// request, and provider signers such as AWS SigV4 hash the body to sign it.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`] if signing fails or if the request ultimately fails
    /// after exhausting retries. A non-success HTTP status code is not itself an
    /// error; inspect [`reqwest::Response::status`] on the returned response.
    pub async fn sign_and_send(&self, request: reqwest::Request) -> Result<reqwest::Response> {
        // The signer operates on a `RequestBuilder`; reconstruct one from the
        // pre-built request (cloning the in-memory body) so the existing
        // sign + retry path applies unchanged.
        let builder = self
            .reqwest_client
            .request(request.method().clone(), request.url().clone());
        let builder = builder.headers(request.headers().clone());
        let builder = match request.body().and_then(|b| b.as_bytes()) {
            Some(bytes) => builder.body(bytes.to_vec()),
            None => builder,
        };
        let builder = self.signer.sign(builder).await?;
        builder
            .send_retry(&self.retry_config, self.service.clone())
            .await
            .map_err(|e| e.error())
    }

    /// Start building a request for the given HTTP `method` and `url`.
    ///
    /// The returned [`CloudRequestBuilder`] borrows this client's signer, so the
    /// request is signed for the configured provider when
    /// [`CloudRequestBuilder::send`] is called.
    pub fn request<U: IntoUrl>(&self, method: Method, url: U) -> CloudRequestBuilder {
        CloudRequestBuilder {
            builder: self.reqwest_client.request(method, url),
            client: self.clone(),
            #[cfg(feature = "recording")]
            out_dir: self.recording.as_ref().map(|r| r.out_dir.clone()),
        }
    }

    /// Start building a `GET` request for `url`. Shortcut for [`request`](Self::request).
    pub fn get<U: IntoUrl>(&self, url: U) -> CloudRequestBuilder {
        self.request(Method::GET, url)
    }

    /// Start building a `POST` request for `url`. Shortcut for [`request`](Self::request).
    pub fn post<U: IntoUrl>(&self, url: U) -> CloudRequestBuilder {
        self.request(Method::POST, url)
    }

    /// Start building a `PUT` request for `url`. Shortcut for [`request`](Self::request).
    pub fn put<U: IntoUrl>(&self, url: U) -> CloudRequestBuilder {
        self.request(Method::PUT, url)
    }

    /// Start building a `DELETE` request for `url`. Shortcut for [`request`](Self::request).
    pub fn delete<U: IntoUrl>(&self, url: U) -> CloudRequestBuilder {
        self.request(Method::DELETE, url)
    }

    /// Start building a `HEAD` request for `url`. Shortcut for [`request`](Self::request).
    pub fn head<U: IntoUrl>(&self, url: U) -> CloudRequestBuilder {
        self.request(Method::HEAD, url)
    }

    /// Start building a `PATCH` request for `url`. Shortcut for [`request`](Self::request).
    pub fn patch<U: IntoUrl>(&self, url: U) -> CloudRequestBuilder {
        self.request(Method::PATCH, url)
    }

    /// Start building an `OPTIONS` request for `url`. Shortcut for [`request`](Self::request).
    pub fn options<U: IntoUrl>(&self, url: U) -> CloudRequestBuilder {
        self.request(Method::OPTIONS, url)
    }

    /// Start building a `TRACE` request for `url`. Shortcut for [`request`](Self::request).
    pub fn trace<U: IntoUrl>(&self, url: U) -> CloudRequestBuilder {
        self.request(Method::TRACE, url)
    }

    /// Start building a `CONNECT` request for `url`. Shortcut for [`request`](Self::request).
    pub fn connect<U: IntoUrl>(&self, url: U) -> CloudRequestBuilder {
        self.request(Method::CONNECT, url)
    }

    /// Enable request/response recording, writing each interaction to `out_dir`.
    ///
    /// Once set, every request sent through this client is captured to a
    /// numbered JSON file (`0000.json`, `0001.json`, …) under `out_dir` for
    /// later test replay. Sensitive response headers (`authorization`,
    /// `x-amz-security-token`, `cookie`, and similar) are replaced with
    /// `"<REDACTED>"` before anything is written to disk, so
    /// recordings never persist bearer tokens or signing secrets. Request
    /// headers are not recorded at all.
    ///
    /// `out_dir` is canonicalized eagerly, so it must already exist.
    ///
    /// Only available when the `recording` feature is enabled.
    ///
    /// # Errors
    ///
    /// Returns the [`io::Error`](std::io::Error) from canonicalizing `out_dir`,
    /// for example if the directory does not exist or is not accessible.
    #[cfg(feature = "recording")]
    pub fn set_recording_dir(&mut self, out_dir: std::path::PathBuf) -> Result<(), std::io::Error> {
        let out_dir = std::fs::canonicalize(out_dir)?;
        self.recording = Some(RecordingState {
            out_dir,
            counter: Arc::new(AtomicU64::new(0)),
        });
        Ok(())
    }
}

/// A builder for a single request issued through a [`CloudClient`].
///
/// Created by [`CloudClient::request`] and the per-verb shortcuts such as
/// [`CloudClient::get`]. Configure the request with the builder methods, then
/// call [`send`](Self::send) to sign and dispatch it.
pub struct CloudRequestBuilder {
    builder: RequestBuilder,
    client: CloudClient,
    #[cfg(feature = "recording")]
    out_dir: Option<PathBuf>,
}

impl CloudRequestBuilder {
    /// Add a `Header` to this Request.
    pub fn header<K, V>(mut self, key: K, value: V) -> CloudRequestBuilder
    where
        HeaderName: TryFrom<K>,
        <HeaderName as TryFrom<K>>::Error: Into<http::Error>,
        HeaderValue: TryFrom<V>,
        <HeaderValue as TryFrom<V>>::Error: Into<http::Error>,
    {
        self.builder = self.builder.header(key, value);
        self
    }

    /// Add a set of Headers to the existing ones on this Request.
    ///
    /// The headers will be merged in to any already set.
    pub fn headers(mut self, headers: HeaderMap) -> CloudRequestBuilder {
        self.builder = self.builder.headers(headers);
        self
    }

    /// Set the request body.
    pub fn body<T: Into<Body>>(mut self, body: T) -> CloudRequestBuilder {
        self.builder = self.builder.body(body);
        self
    }

    /// Enables a request timeout.
    ///
    /// The timeout is applied from when the request starts connecting until the
    /// response body has finished. It affects only this request and overrides
    /// the timeout configured using `ClientBuilder::timeout()`.
    pub fn timeout(mut self, timeout: Duration) -> CloudRequestBuilder {
        self.builder = self.builder.timeout(timeout);
        self
    }

    /// Modify the query string of the URL.
    ///
    /// Modifies the URL of this request, adding the parameters provided.
    /// This method appends and does not overwrite. This means that it can
    /// be called multiple times and that existing query parameters are not
    /// overwritten if the same key is used. The key will simply show up
    /// twice in the query string.
    /// Calling `.query(&[("foo", "a"), ("foo", "b")])` gives `"foo=a&foo=b"`.
    ///
    /// # Note
    /// This method does not support serializing a single key-value
    /// pair. Instead of using `.query(("key", "val"))`, use a sequence, such
    /// as `.query(&[("key", "val")])`. It's also possible to serialize structs
    /// and maps into a key-value pair.
    ///
    /// # Errors
    /// This method will fail if the object you provide cannot be serialized
    /// into a query string.
    pub fn query<T: Serialize + ?Sized>(mut self, query: &T) -> CloudRequestBuilder {
        self.builder = self.builder.query(query);
        self
    }

    /// Send a JSON body.
    ///
    /// # Errors
    ///
    /// Serialization can fail if `T`'s implementation of `Serialize` decides to
    /// fail, or if `T` contains a map with non-string keys.
    pub fn json<T: Serialize + ?Sized>(mut self, json: &T) -> CloudRequestBuilder {
        self.builder = self.builder.json(json);
        self
    }

    /// Sign and send the request, returning the [`reqwest::Response`].
    ///
    /// The request is first passed to the client's [`RequestSigner`], which
    /// attaches the provider-specific authentication (e.g. AWS SigV4 headers or
    /// an `Authorization: Bearer` header), refreshing any cached credential as
    /// needed. The signed request is then dispatched through the client's
    /// [`HttpService`].
    ///
    /// When the `recording` feature is enabled and a recording directory has
    /// been configured via `CloudClient::set_recording_dir`, the request and
    /// response are also written to disk (with sensitive headers redacted)
    /// before the response is returned to the caller.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`] if signing fails, if the request cannot be built, or
    /// if the underlying HTTP transport returns an error. A non-success HTTP
    /// status code is not itself an error; inspect [`reqwest::Response::status`]
    /// on the returned response.
    pub async fn send(mut self) -> Result<reqwest::Response> {
        self.builder = self.client.signer.sign(self.builder).await?;

        #[cfg(not(feature = "recording"))]
        {
            self.builder
                .send_retry(&self.client.retry_config, self.client.service.clone())
                .await
                .map_err(|e| e.error())
        }
        #[cfg(feature = "recording")]
        {
            let response = send_record(self).await?;
            Ok(response)
        }
    }
}

#[cfg(feature = "recording")]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RequestResponseInfo {
    pub request: RequestInfo,
    pub response: ResponseInfo,
}

#[cfg(feature = "recording")]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RequestInfo {
    pub method: String,
    pub url_path: String,
    pub body: Option<String>,
}

#[cfg(feature = "recording")]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ResponseInfo {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: Option<String>,
}

/// Header names whose values must never appear in recording files.
///
/// These headers carry bearer tokens, signing secrets, or session tokens.
/// They are replaced with `"<REDACTED>"` before any recording is written to disk.
#[cfg(feature = "recording")]
const SENSITIVE_HEADERS: &[&str] = &[
    "authorization",
    "x-amz-security-token",
    "x-amz-content-sha256",
    "x-databricks-authorization",
    "x-ms-identity-principal-id",
    "x-goog-iam-credentials-token",
    "cookie",
    "set-cookie",
];

#[cfg(feature = "recording")]
fn redact_headers(headers: &HashMap<String, String>) -> HashMap<String, String> {
    headers
        .iter()
        .map(|(k, v)| {
            let v = if SENSITIVE_HEADERS.contains(&k.to_lowercase().as_str()) {
                "<REDACTED>".to_string()
            } else {
                v.clone()
            };
            (k.clone(), v)
        })
        .collect()
}

#[cfg(feature = "recording")]
async fn send_record(builder: CloudRequestBuilder) -> Result<reqwest::Response> {
    let Some(out_dir) = builder.out_dir else {
        let request = builder.builder.build().expect("request to be valid");
        return builder.client.service.call(request).await;
    };
    let (_client, request) = builder.builder.build_split();
    let request = request.expect("request to be valid");

    let request_info = RequestInfo {
        method: request.method().as_str().to_string(),
        url_path: {
            let url = request.url();
            match url.query() {
                Some(query) => format!("{}?{}", url.path(), query),
                None => url.path().to_string(),
            }
        },
        body: request
            .body()
            .and_then(|b| b.as_bytes().map(|b| String::from_utf8_lossy(b).to_string())),
    };

    let response = builder.client.service.call(request).await?;

    // Record the response
    let status = response.status().as_u16();
    let raw_headers: HashMap<String, String> = response
        .headers()
        .iter()
        .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();

    // Get response body while preserving it for the caller
    let response_bytes = response.bytes().await?;
    let response_body = if response_bytes.is_empty() {
        None
    } else {
        Some(String::from_utf8_lossy(&response_bytes).to_string())
    };

    let recording = RequestResponseInfo {
        request: request_info,
        response: ResponseInfo {
            status,
            // Redact sensitive headers before writing to disk
            headers: redact_headers(&raw_headers),
            body: response_body,
        },
    };

    let counter = builder
        .client
        .recording
        .as_ref()
        .map(|r| r.counter.fetch_add(1, Ordering::SeqCst))
        .unwrap_or(0);
    let file_path = out_dir.join(format!("{:04}.json", counter));
    if let Err(e) = std::fs::File::create(&file_path)
        .and_then(|f| serde_json::to_writer_pretty(f, &recording).map_err(Into::into))
    {
        tracing::warn!(
            "Failed to write recording to {}: {}",
            file_path.display(),
            e
        );
    }

    // Return a new response built from the recorded data, using the raw (unredacted) headers
    let mut mock_response = http::Response::builder().status(status);
    for (k, v) in &raw_headers {
        mock_response = mock_response.header(k, v);
    }
    let mock_response = mock_response
        .body(response_bytes)
        .expect("valid status code and headers");

    Ok(reqwest::Response::from(mock_response))
}

#[cfg(all(test, feature = "recording"))]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_request_response_recording() {
        // Create a temporary directory for recordings
        let temp_dir = TempDir::new().unwrap();
        let temp_path = temp_dir.path().to_path_buf();

        // Set up a mock server
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/test")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"message": "Hello, World!"}"#)
            .create_async()
            .await;

        // Create a cloud client with recording enabled
        let mut client = CloudClient::new_unauthenticated();
        client.set_recording_dir(temp_path.clone()).unwrap();

        // Make a request
        let url = format!("{}/test", server.url());
        let response = client.get(&url).send().await.unwrap();

        // Verify the response is correct
        assert_eq!(response.status(), 200);
        let body = response.text().await.unwrap();
        assert_eq!(body, r#"{"message": "Hello, World!"}"#);

        // Verify that a recording file was created
        let recordings: Vec<_> = fs::read_dir(&temp_path)
            .unwrap()
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let path = entry.path();
                if path.extension()? == "json" {
                    Some(path)
                } else {
                    None
                }
            })
            .collect();

        assert_eq!(recordings.len(), 1, "Expected exactly one recording file");

        // Read and verify the recording content
        let recording_content = fs::read_to_string(&recordings[0]).unwrap();
        let recording: RequestResponseInfo = serde_json::from_str(&recording_content).unwrap();

        // Verify request information
        assert_eq!(recording.request.method, "GET");
        assert_eq!(recording.request.url_path, "/test");
        assert_eq!(recording.request.body, None);

        // Verify response information
        assert_eq!(recording.response.status, 200);
        assert_eq!(
            recording.response.headers.get("content-type").unwrap(),
            "application/json"
        );
        assert_eq!(
            recording.response.body.as_ref().unwrap(),
            r#"{"message": "Hello, World!"}"#
        );

        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_recording_with_request_body() {
        // Create a temporary directory for recordings
        let temp_dir = TempDir::new().unwrap();
        let temp_path = temp_dir.path().to_path_buf();

        // Set up a mock server
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/create")
            .with_status(201)
            .with_header("location", "/resource/123")
            .with_body(r#"{"id": 123, "status": "created"}"#)
            .create_async()
            .await;

        // Create a cloud client with recording enabled
        let mut client = CloudClient::new_unauthenticated();
        client.set_recording_dir(temp_path.clone()).unwrap();

        // Make a POST request with body
        let url = format!("{}/create", server.url());
        let response = client
            .post(&url)
            .json(&serde_json::json!({"name": "test resource"}))
            .send()
            .await
            .unwrap();

        // Verify the response
        assert_eq!(response.status(), 201);

        // Verify that a recording file was created
        let recordings: Vec<_> = fs::read_dir(&temp_path)
            .unwrap()
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let path = entry.path();
                if path.extension()? == "json" {
                    Some(path)
                } else {
                    None
                }
            })
            .collect();

        assert_eq!(recordings.len(), 1);

        // Read and verify the recording content
        let recording_content = fs::read_to_string(&recordings[0]).unwrap();
        let recording: RequestResponseInfo = serde_json::from_str(&recording_content).unwrap();

        // Verify request information
        assert_eq!(recording.request.method, "POST");
        assert_eq!(recording.request.url_path, "/create");
        assert!(recording.request.body.is_some());
        assert!(recording.request.body.unwrap().contains("test resource"));

        // Verify response information
        assert_eq!(recording.response.status, 201);
        assert_eq!(
            recording.response.headers.get("location").unwrap(),
            "/resource/123"
        );
        assert!(recording.response.body.unwrap().contains("created"));

        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_counter_based_file_naming() {
        // Create a temporary directory for recordings
        let temp_dir = TempDir::new().unwrap();
        let temp_path = temp_dir.path().to_path_buf();

        // Set up a mock server
        let mut server = mockito::Server::new_async().await;
        let mock1 = server
            .mock("GET", "/first")
            .with_status(200)
            .with_body("first response")
            .create_async()
            .await;
        let mock2 = server
            .mock("GET", "/second")
            .with_status(200)
            .with_body("second response")
            .create_async()
            .await;

        // Create a cloud client with recording enabled
        let mut client = CloudClient::new_unauthenticated();
        client.set_recording_dir(temp_path.clone()).unwrap();

        // Make multiple requests
        let url1 = format!("{}/first", server.url());
        let url2 = format!("{}/second", server.url());

        let _response1 = client.get(&url1).send().await.unwrap();
        let _response2 = client.get(&url2).send().await.unwrap();

        // Verify that files are named with incrementing counter
        let mut recordings: Vec<_> = fs::read_dir(&temp_path)
            .unwrap()
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let path = entry.path();
                if path.extension()? == "json" {
                    Some(path)
                } else {
                    None
                }
            })
            .collect();

        recordings.sort();
        assert_eq!(recordings.len(), 2);

        // Check that files are named 000000.json and 000001.json
        assert!(recordings[0].file_name().unwrap().to_str().unwrap() == "0000.json");
        assert!(recordings[1].file_name().unwrap().to_str().unwrap() == "0001.json");

        // Verify content matches the order of requests
        let first_content = fs::read_to_string(&recordings[0]).unwrap();
        let first_recording: RequestResponseInfo = serde_json::from_str(&first_content).unwrap();
        assert_eq!(first_recording.request.url_path, "/first");
        assert_eq!(
            first_recording.response.body.as_ref().unwrap(),
            "first response"
        );

        let second_content = fs::read_to_string(&recordings[1]).unwrap();
        let second_recording: RequestResponseInfo = serde_json::from_str(&second_content).unwrap();
        assert_eq!(second_recording.request.url_path, "/second");
        assert_eq!(
            second_recording.response.body.as_ref().unwrap(),
            "second response"
        );

        mock1.assert_async().await;
        mock2.assert_async().await;
    }

    #[tokio::test]
    async fn test_query_parameter_recording() {
        // Create a temporary directory for recordings
        let temp_dir = TempDir::new().unwrap();
        let temp_path = temp_dir.path().to_path_buf();

        // Start a mock server
        let mut server = mockito::Server::new_async().await;

        // Create a mock that expects query parameters
        let mock = server
            .mock("GET", "/catalogs?max_results=10&page_token=abc123")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"catalogs": []}"#)
            .create_async()
            .await;

        // Create a client with recording enabled
        let mut client = CloudClient::new_unauthenticated();
        client.set_recording_dir(temp_path.clone()).unwrap();

        // Make a request with query parameters
        let url = format!("{}/catalogs?max_results=10&page_token=abc123", server.url());
        let response = client.get(&url).send().await.unwrap();

        assert!(response.status().is_success());

        // Verify that the recording file was created
        let recordings: Vec<_> = fs::read_dir(&temp_path)
            .unwrap()
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let path = entry.path();
                if path.extension()? == "json" {
                    Some(path)
                } else {
                    None
                }
            })
            .collect();

        assert_eq!(recordings.len(), 1);

        // Read and verify the recording content includes query parameters
        let recording_content = fs::read_to_string(&recordings[0]).unwrap();
        let recording: RequestResponseInfo = serde_json::from_str(&recording_content).unwrap();

        // Verify request information includes query parameters
        assert_eq!(recording.request.method, "GET");
        assert_eq!(
            recording.request.url_path,
            "/catalogs?max_results=10&page_token=abc123"
        );
        assert_eq!(recording.request.body, None);

        // Verify response information
        assert_eq!(recording.response.status, 200);
        assert_eq!(
            recording.response.body.as_ref().unwrap(),
            r#"{"catalogs": []}"#
        );

        mock.assert_async().await;
    }

    /// Verify that the bearer token injected by a signed request does not appear
    /// in the recording file. Request headers are currently not recorded, so this
    /// test confirms the token does not leak via any other path (e.g. echoed back
    /// in a response header or response body).
    #[tokio::test]
    async fn test_recording_does_not_contain_bearer_token_value() {
        let temp_dir = TempDir::new().unwrap();
        let temp_path = temp_dir.path().to_path_buf();

        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/secret")
            .match_header("authorization", mockito::Matcher::Any)
            .with_status(200)
            // Server does NOT echo the token back — simulates a well-behaved API
            .with_body("ok")
            .create_async()
            .await;

        let mut client = CloudClient::new_with_token("super-secret-token-12345");
        client.set_recording_dir(temp_path.clone()).unwrap();

        let url = format!("{}/secret", server.url());
        client.get(&url).send().await.unwrap();

        let recording_path = temp_path.join("0000.json");
        let content = fs::read_to_string(&recording_path).unwrap();

        // The raw token must not appear in the file at all
        assert!(
            !content.contains("super-secret-token-12345"),
            "raw bearer token leaked into recording: {content}"
        );

        mock.assert_async().await;
    }

    /// Verify that sensitive headers returned by the server (e.g. a reflected
    /// Authorization or AWS security token) are redacted before being written
    /// to the recording file.
    #[tokio::test]
    async fn test_recording_redacts_sensitive_response_headers() {
        let temp_dir = TempDir::new().unwrap();
        let temp_path = temp_dir.path().to_path_buf();

        let mut server = mockito::Server::new_async().await;
        // Simulate a server that echoes a sensitive header back in its response
        let mock = server
            .mock("GET", "/s3")
            .with_status(200)
            .with_header("x-amz-security-token", "AQoXnyc4LLI2AJvUAMOGAR8a1234567890")
            .with_header("authorization", "Bearer should-be-redacted")
            .with_body("{}")
            .create_async()
            .await;

        let mut client = CloudClient::new_unauthenticated();
        client.set_recording_dir(temp_path.clone()).unwrap();

        let url = format!("{}/s3", server.url());
        client.get(&url).send().await.unwrap();

        let content = fs::read_to_string(temp_path.join("0000.json")).unwrap();
        assert!(
            !content.contains("AQoXnyc4LLI2AJvUAMOGAR8a1234567890"),
            "x-amz-security-token leaked into recording: {content}"
        );
        assert!(
            !content.contains("should-be-redacted"),
            "Authorization value leaked into recording: {content}"
        );
        // Both headers should be replaced with the redaction sentinel
        assert_eq!(
            content.matches("<REDACTED>").count(),
            2,
            "expected 2 <REDACTED> entries in recording: {content}"
        );

        mock.assert_async().await;
    }

    /// Verify that a recording produced with redacted headers can still be parsed.
    #[tokio::test]
    async fn test_recording_remains_valid_json_after_redaction() {
        let temp_dir = TempDir::new().unwrap();
        let temp_path = temp_dir.path().to_path_buf();

        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/check")
            .with_status(200)
            .with_header("authorization", "Bearer should-be-redacted")
            .with_body(r#"{"ok":true}"#)
            .create_async()
            .await;

        let mut client = CloudClient::new_unauthenticated();
        client.set_recording_dir(temp_path.clone()).unwrap();

        let url = format!("{}/check", server.url());
        client.get(&url).send().await.unwrap();

        let content = fs::read_to_string(temp_path.join("0000.json")).unwrap();
        // Must parse cleanly as a RequestResponseInfo
        let parsed: RequestResponseInfo = serde_json::from_str(&content)
            .expect("recording file must be valid JSON even after redaction");
        assert_eq!(parsed.response.status, 200);
    }
}

#[cfg(test)]
mod retry_integration_tests {
    use super::*;
    use std::time::Duration;

    fn fast_retry(max_retries: usize) -> RetryConfig {
        RetryConfig {
            backoff: crate::backoff::BackoffConfig {
                init_backoff: Duration::from_millis(1),
                max_backoff: Duration::from_millis(5),
                base: 2.,
            },
            max_retries,
            retry_timeout: Duration::from_secs(30),
        }
    }

    // A 5xx is retried by CloudClient::send: a single 503 followed by a 200
    // should surface the 200, proving the user-request path now honors
    // retry_config (previously it did a bare service.call with no retry).
    #[tokio::test]
    async fn send_retries_server_error_then_succeeds() {
        let mut server = mockito::Server::new_async().await;
        let fail = server
            .mock("GET", "/r")
            .with_status(503)
            .expect(1)
            .create_async()
            .await;
        let ok = server
            .mock("GET", "/r")
            .with_status(200)
            .with_body("ok")
            .expect(1)
            .create_async()
            .await;

        let client = CloudClient::new_unauthenticated().with_retry_config(fast_retry(3));
        let resp = client
            .get(format!("{}/r", server.url()))
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), 200);
        assert_eq!(resp.text().await.unwrap(), "ok");
        fail.assert_async().await;
        ok.assert_async().await;
    }

    // A 4xx is not retryable: it must surface immediately as an error without
    // consuming retries (a second mock would go unmatched).
    #[tokio::test]
    async fn send_does_not_retry_client_error() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/r")
            .with_status(404)
            .expect(1)
            .create_async()
            .await;

        let client = CloudClient::new_unauthenticated().with_retry_config(fast_retry(3));
        let err = client
            .get(format!("{}/r", server.url()))
            .send()
            .await
            .unwrap_err();

        assert!(matches!(err, Error::NotFound { .. }), "got {err:?}");
        mock.assert_async().await;
    }

    // sign_and_send takes a pre-built reqwest::Request, applies the signer
    // (here a bearer token), and retries transient failures just like send.
    #[tokio::test]
    async fn sign_and_send_signs_and_retries() {
        let mut server = mockito::Server::new_async().await;
        let fail = server
            .mock("POST", "/rpc")
            .match_header("authorization", "Bearer tok")
            .with_status(503)
            .expect(1)
            .create_async()
            .await;
        let ok = server
            .mock("POST", "/rpc")
            .match_header("authorization", "Bearer tok")
            .with_status(200)
            .with_body("pong")
            .expect(1)
            .create_async()
            .await;

        let client = CloudClient::new_with_token("tok").with_retry_config(fast_retry(3));
        let request = reqwest::Client::new()
            .post(format!("{}/rpc", server.url()))
            .body("ping")
            .build()
            .unwrap();
        let resp = client.sign_and_send(request).await.unwrap();

        assert_eq!(resp.status(), 200);
        assert_eq!(resp.text().await.unwrap(), "pong");
        fail.assert_async().await;
        ok.assert_async().await;
    }
}
