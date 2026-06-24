//! Browser/WASM HTTP transport for `olai-codegen`-generated clients.
//!
//! This is the WASM counterpart to `olai_http::CloudClient`. Generated client bodies call
//! `self.client.<verb>(url).json(req).query(..).send().await?` and then `response.status()` /
//! `response.bytes()`. [`WasmClient`] and [`WasmRequestBuilder`] expose exactly that surface, so
//! the *same* generated code compiles against either transport — selected at generation time via
//! `CodeGenConfig::transport_type_path` (see `olai-codegen`).
//!
//! Unlike `CloudClient`, there is no request signing, no `ring`/`tokio`/`hyper`, and no cloud
//! credential discovery. In the browser the **session is managed by the browser**: by default we
//! ask `fetch` to include credentials (`RequestCredentials::Include`) so cookies / auth headers
//! ride along automatically. This keeps the wasm dependency tree minimal and is the right model
//! for a front-end SDK talking to a same-origin (or CORS-with-credentials) API.
//!
//! Some embedders manage auth themselves — e.g. the host app already holds a bearer token and
//! wants to attach `Authorization: Bearer …` per request, possibly refreshing it as it rotates.
//! For those, the browser is *not* the session of record, so two knobs extend (rather than
//! replace) the browser-session model without pulling in cloud-credential signing:
//!
//! - [`WasmClient::with_auth`] installs a cheaply-cloneable, per-request header provider. It is
//!   invoked just before each `send`, so a token that rotates is always read fresh.
//! - [`WasmClient::with_credentials`] chooses the `fetch` credentials mode
//!   ([`CredentialsMode`]). It defaults to [`Include`](CredentialsMode::Include) (cookie mode);
//!   a bearer-token embedder typically sets [`Omit`](CredentialsMode::Omit) so a stale cookie
//!   doesn't shadow the header. Setting an auth hook does **not** silently change the credentials
//!   mode — the two are independent so cookie mode keeps working.
//!
//! On a native target the credentials mode is a no-op (there is no browser session), but the auth
//! header provider still applies — so the crate builds and the header behavior can be unit-tested
//! off-wasm.
//!
//! # Examples
//!
//! Drive the same verb -> body -> query -> send chain that generated client bodies use. This
//! reaches the network and (on wasm) the browser session, so it is marked `no_run`:
//!
//! ```no_run
//! # async fn run() -> reqwest::Result<()> {
//! use olai_http_wasm::WasmClient;
//!
//! let client = WasmClient::new();
//! let response = client
//!     .get("https://example.test/api/things")
//!     .query(&[("page_size", "10")])
//!     .send()
//!     .await?;
//! let _status = response.status();
//! # Ok(())
//! # }
//! ```

use std::fmt;
use std::sync::Arc;

use reqwest::header::HeaderMap;
use reqwest::{Client, IntoUrl, Method};
use serde::Serialize;

/// Re-export of the `reqwest` this crate is built against.
///
/// Generated bindings build an auth [`HeaderMap`] (e.g. `Authorization: Bearer …`) and reach the
/// header types through this re-export, so they don't need their own `reqwest` dependency and
/// stay pinned to the same wasm-flavored reqwest as the transport.
pub use reqwest;

/// A cheaply-cloneable, per-request header provider.
///
/// Installed via [`WasmClient::with_auth`] and invoked just before each [`WasmRequestBuilder::send`],
/// so a rotating token is read fresh on every request. Cloning the [`WasmClient`] only bumps an
/// `Arc` refcount.
type AuthHook = Arc<dyn Fn() -> HeaderMap + Send + Sync>;

/// How `fetch` should treat the browser-managed session (cookies / auth headers).
///
/// Mirrors the [`Request.credentials`][mdn] values. Only meaningful on wasm; on a native target
/// there is no browser session and the mode is a no-op.
///
/// [mdn]: https://developer.mozilla.org/en-US/docs/Web/API/Request/credentials
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CredentialsMode {
    /// Always send the browser session (cookies / auth headers), even cross-origin. This is the
    /// default and the cookie-auth mode the crate was built around.
    #[default]
    Include,
    /// Send the browser session only for same-origin requests.
    SameOrigin,
    /// Never send the browser session. Typical for bearer-token embedders, where a stale cookie
    /// should not shadow an explicit `Authorization` header.
    Omit,
}

/// Browser/WASM HTTP transport. Cheap to clone (wraps a [`reqwest::Client`]).
///
/// Mirrors the verb surface of `olai_http::CloudClient` (`get`/`post`/`put`/`patch`/`delete`)
/// so it is a drop-in transport for generated clients.
///
/// By default it runs in browser-session (cookie) mode. Use [`with_auth`](Self::with_auth) to
/// attach a per-request `Authorization` header and [`with_credentials`](Self::with_credentials)
/// to choose the `fetch` credentials mode.
#[derive(Clone)]
pub struct WasmClient {
    client: Client,
    auth: Option<AuthHook>,
    credentials: CredentialsMode,
}

impl fmt::Debug for WasmClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The auth hook is an opaque closure (no Debug) and may close over a secret-bearing token,
        // so render only whether one is installed — never its contents.
        f.debug_struct("WasmClient")
            .field("client", &self.client)
            .field("auth", &self.auth.as_ref().map(|_| "<fn>"))
            .field("credentials", &self.credentials)
            .finish()
    }
}

impl Default for WasmClient {
    fn default() -> Self {
        Self::new()
    }
}

impl WasmClient {
    /// Create a new browser transport in browser-session (cookie) mode.
    ///
    /// Requests run in the browser session: [`WasmRequestBuilder::send`] asks `fetch` to include
    /// credentials ([`CredentialsMode::Include`]). On a native target there is no browser session,
    /// so that credentials step is a no-op and requests are sent as built.
    ///
    /// For bearer-token auth, chain [`with_auth`](Self::with_auth) (and usually
    /// [`with_credentials(CredentialsMode::Omit)`](Self::with_credentials)).
    pub fn new() -> Self {
        Self {
            client: Client::new(),
            auth: None,
            credentials: CredentialsMode::default(),
        }
    }

    /// Builds a transport from an existing [`reqwest::Client`] (e.g. one configured elsewhere).
    pub fn from_client(client: Client) -> Self {
        Self {
            client,
            auth: None,
            credentials: CredentialsMode::default(),
        }
    }

    /// Install a per-request header provider (e.g. bearer-token auth).
    ///
    /// `hook` is invoked just before each [`WasmRequestBuilder::send`] and its [`HeaderMap`] is
    /// merged onto the outgoing request, so a token that rotates is always read fresh. The hook is
    /// stored behind an [`Arc`], so cloning the [`WasmClient`] stays cheap.
    ///
    /// This is independent of [`with_credentials`](Self::with_credentials): setting an auth hook
    /// does not change the `fetch` credentials mode. A bearer-token embedder typically pairs this
    /// with [`with_credentials(CredentialsMode::Omit)`](Self::with_credentials) so a stale cookie
    /// does not shadow the header.
    ///
    /// The hook is synchronous. If a future revision needs async token refresh (e.g. awaiting a
    /// refresh-token exchange), it can be added as a sibling builder taking an async closure
    /// without changing this signature.
    ///
    /// ```no_run
    /// # use olai_http_wasm::{WasmClient, CredentialsMode};
    /// use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
    ///
    /// let token = "secret-token".to_string();
    /// let client = WasmClient::new()
    ///     .with_credentials(CredentialsMode::Omit)
    ///     .with_auth(move || {
    ///         let mut headers = HeaderMap::new();
    ///         if let Ok(value) = HeaderValue::from_str(&format!("Bearer {token}")) {
    ///             headers.insert(AUTHORIZATION, value);
    ///         }
    ///         headers
    ///     });
    /// ```
    pub fn with_auth<F>(mut self, hook: F) -> Self
    where
        F: Fn() -> HeaderMap + Send + Sync + 'static,
    {
        self.auth = Some(Arc::new(hook));
        self
    }

    /// Choose the `fetch` credentials mode (cookie behavior). Defaults to
    /// [`CredentialsMode::Include`]. No-op on a native target.
    pub fn with_credentials(mut self, credentials: CredentialsMode) -> Self {
        self.credentials = credentials;
        self
    }

    fn request<U: IntoUrl>(&self, method: Method, url: U) -> WasmRequestBuilder {
        WasmRequestBuilder {
            builder: self.client.request(method, url),
            auth: self.auth.clone(),
            credentials: self.credentials,
        }
    }

    /// Starts a `GET` request to `url`.
    pub fn get<U: IntoUrl>(&self, url: U) -> WasmRequestBuilder {
        self.request(Method::GET, url)
    }

    /// Starts a `POST` request to `url`.
    pub fn post<U: IntoUrl>(&self, url: U) -> WasmRequestBuilder {
        self.request(Method::POST, url)
    }

    /// Starts a `PUT` request to `url`.
    pub fn put<U: IntoUrl>(&self, url: U) -> WasmRequestBuilder {
        self.request(Method::PUT, url)
    }

    /// Starts a `PATCH` request to `url`.
    pub fn patch<U: IntoUrl>(&self, url: U) -> WasmRequestBuilder {
        self.request(Method::PATCH, url)
    }

    /// Starts a `DELETE` request to `url`.
    pub fn delete<U: IntoUrl>(&self, url: U) -> WasmRequestBuilder {
        self.request(Method::DELETE, url)
    }
}

/// A single in-flight request. Mirrors the slice of `olai_http::CloudRequestBuilder` that
/// generated client bodies use (`json`, `query`, `send`).
pub struct WasmRequestBuilder {
    builder: reqwest::RequestBuilder,
    auth: Option<AuthHook>,
    credentials: CredentialsMode,
}

impl WasmRequestBuilder {
    /// Set a JSON request body.
    ///
    /// If `json` fails to serialize, the error is stored and surfaced later by
    /// [`send`](Self::send) rather than reported here.
    pub fn json<T: Serialize + ?Sized>(mut self, json: &T) -> WasmRequestBuilder {
        self.builder = self.builder.json(json);
        self
    }

    /// Append query parameters to the URL.
    ///
    /// If `query` fails to serialize, the error is stored and surfaced later by
    /// [`send`](Self::send) rather than reported here.
    pub fn query<T: Serialize + ?Sized>(mut self, query: &T) -> WasmRequestBuilder {
        self.builder = self.builder.query(query);
        self
    }

    /// Sends the request and returns the [`reqwest::Response`].
    ///
    /// Just before sending, the auth hook (if one was installed via [`WasmClient::with_auth`]) is
    /// invoked and its headers are merged onto the request — so a rotating token is read fresh.
    /// On wasm we then apply the [`CredentialsMode`] so the browser attaches (or withholds) its
    /// session. On native there is no browser session, so the credentials step is a no-op and the
    /// request is sent as built (useful for off-wasm unit tests); the auth headers still apply.
    ///
    /// Returns a [`reqwest::Result`] exactly like reqwest, so generated bodies' `?` and the
    /// project's `parse_error_response(response)` / `response.bytes()` continue to work unchanged.
    ///
    /// # Errors
    ///
    /// Returns an error if the request cannot be completed, including:
    ///
    /// - a network failure (the host is unreachable, the connection drops, or the request
    ///   times out);
    /// - in the browser, a `fetch` rejection such as a CORS policy denial;
    /// - a serialization error deferred from [`json`](Self::json) or [`query`](Self::query),
    ///   which is held until `send` is called.
    pub async fn send(self) -> reqwest::Result<reqwest::Response> {
        self.with_auth_headers()
            .with_browser_credentials()
            .builder
            .send()
            .await
    }

    /// Invoke the auth hook (if any) and merge its headers onto the request. Applies on every
    /// target so off-wasm unit tests exercise the same header logic.
    fn with_auth_headers(mut self) -> Self {
        if let Some(auth) = &self.auth {
            self.builder = self.builder.headers(auth());
        }
        self
    }

    #[cfg(target_arch = "wasm32")]
    fn with_browser_credentials(mut self) -> Self {
        // reqwest's wasm builder exposes one method per credentials mode rather than a setter.
        self.builder = match self.credentials {
            CredentialsMode::Include => self.builder.fetch_credentials_include(),
            CredentialsMode::SameOrigin => self.builder.fetch_credentials_same_origin(),
            CredentialsMode::Omit => self.builder.fetch_credentials_omit(),
        };
        self
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn with_browser_credentials(self) -> Self {
        let _ = self.credentials;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;
    use url::Url;

    #[derive(Serialize)]
    struct CreateThing {
        name: String,
    }

    /// Compile-time contract: a `WasmClient` drives the exact builder/response chain that
    /// `olai-codegen` emits (`post(url).json(req).query(..).send()`), so generated client bodies
    /// compile against this transport unchanged. The request is built but not sent (no network).
    #[test]
    fn drives_generated_client_chain() {
        let client = WasmClient::new();
        let base = Url::parse("https://example.test/api/").unwrap();
        let url = base.join("things").unwrap();
        let req = CreateThing {
            name: "x".to_string(),
        };

        // Mirrors generated bodies: verb -> json -> query -> (send). We assert the builder
        // chains without panicking; awaiting send() would require a runtime/network.
        let _builder = client
            .post(url.clone())
            .json(&req)
            .query(&[("dry_run", "true")]);
        let _get = client.get(url.clone());
        let _patch = client.patch(url.clone());
        let _delete = client.delete(url.clone());
        let _put = client.put(url);
    }

    #[test]
    fn clone_is_cheap_and_default_works() {
        let client = WasmClient::default();
        let _c2 = client.clone();
    }

    /// An auth hook builds cleanly and survives a clone (the `Arc` is shared, not the closure).
    /// We can't observe the merged headers without sending, but on native `with_auth_headers`
    /// runs the same code path, so building the request must not panic.
    #[test]
    fn with_auth_hook_builds_and_clones() {
        use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};

        let client = WasmClient::new()
            .with_credentials(CredentialsMode::Omit)
            .with_auth(|| {
                let mut headers = HeaderMap::new();
                headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer test-token"));
                headers
            });
        let cloned = client.clone();

        let url = Url::parse("https://example.test/api/things").unwrap();
        // Apply the auth hook the same way `send` does; this exercises the merge without network.
        let _req = cloned.get(url).with_auth_headers();
    }

    /// The default credentials mode is cookie (`Include`) mode, matching the original behavior so
    /// existing callers are unchanged.
    #[test]
    fn default_credentials_mode_is_include() {
        assert_eq!(CredentialsMode::default(), CredentialsMode::Include);
    }

    /// Debug must not render the auth closure's contents (it may close over a secret token).
    #[test]
    fn debug_redacts_auth_hook() {
        let client = WasmClient::new().with_auth(reqwest::header::HeaderMap::new);
        let rendered = format!("{client:?}");
        assert!(
            rendered.contains("auth"),
            "Debug should note the auth field"
        );
        assert!(
            rendered.contains("<fn>"),
            "Debug should render the hook opaquely"
        );
    }
}
