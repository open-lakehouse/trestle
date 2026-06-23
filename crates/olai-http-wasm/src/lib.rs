//! Browser/WASM HTTP transport for `olai-codegen`-generated clients.
//!
//! This is the WASM counterpart to `olai_http::CloudClient`. Generated client bodies call
//! `self.client.<verb>(url).json(req).query(..).send().await?` and then `response.status()` /
//! `response.bytes()`. [`WasmClient`] and [`WasmRequestBuilder`] expose exactly that surface, so
//! the *same* generated code compiles against either transport — selected at generation time via
//! `CodeGenConfig::transport_type_path` (see `olai-codegen`).
//!
//! Unlike `CloudClient`, there is no request signing, no `ring`/`tokio`/`hyper`, and no cloud
//! credential discovery. In the browser the **session is managed by the browser**: we ask `fetch`
//! to include credentials (`RequestCredentials::Include`) so cookies / auth headers ride along
//! automatically. This keeps the wasm dependency tree minimal and is the right model for a
//! front-end SDK talking to a same-origin (or CORS-with-credentials) API.
//!
//! On a native target the credential step is a no-op (there is no browser session), so the crate
//! still builds and can be unit-tested off-wasm.
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

use reqwest::{Client, IntoUrl, Method};
use serde::Serialize;

/// Browser/WASM HTTP transport. Cheap to clone (wraps a [`reqwest::Client`]).
///
/// Mirrors the verb surface of `olai_http::CloudClient` (`get`/`post`/`put`/`patch`/`delete`)
/// so it is a drop-in transport for generated clients.
#[derive(Clone, Debug)]
pub struct WasmClient {
    client: Client,
}

impl Default for WasmClient {
    fn default() -> Self {
        Self::new()
    }
}

impl WasmClient {
    /// Create a new browser transport.
    ///
    /// The browser attaches the session, so there is no token/credential variant — unlike
    /// `CloudClient::new_with_token` / `new_unauthenticated`. Requests run in the browser session:
    /// [`WasmRequestBuilder::send`] asks `fetch` to include credentials. On a native target there
    /// is no browser session, so that credential step is a no-op and requests are sent as built.
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    /// Builds a transport from an existing [`reqwest::Client`] (e.g. one configured elsewhere).
    pub fn from_client(client: Client) -> Self {
        Self { client }
    }

    fn request<U: IntoUrl>(&self, method: Method, url: U) -> WasmRequestBuilder {
        WasmRequestBuilder {
            builder: self.client.request(method, url),
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
    /// On wasm we set `credentials: "include"` so the browser attaches its session (cookies /
    /// auth headers). On native there is no browser session, so this is a no-op and the request
    /// is sent as built (useful for off-wasm unit tests).
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
        self.with_browser_credentials().builder.send().await
    }

    #[cfg(target_arch = "wasm32")]
    fn with_browser_credentials(mut self) -> Self {
        self.builder = self.builder.fetch_credentials_include();
        self
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn with_browser_credentials(self) -> Self {
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
}
