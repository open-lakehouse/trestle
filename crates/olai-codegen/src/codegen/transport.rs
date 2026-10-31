//! The plain `reqwest` transport of generated REST clients, selected with
//! `transport_type_path =` [`REQWEST_TRANSPORT_TYPE_PATH`]: a `reqwest::Client` with an optional
//! bearer token and `User-Agent`. No credential providers, no retries; builds for `wasm32` too.

use quote::quote;

use super::format_tokens_static;

/// Transport type path that selects the generated `reqwest` transport.
pub const REQWEST_TRANSPORT_TYPE_PATH: &str = "crate::codegen::transport::HttpClient";

/// The `transport.rs` file. With `user_agent`, the default transport sends
/// `<user_agent>/<client crate version>`, except in the browser.
pub(crate) fn generate(user_agent: Option<&str>) -> String {
    let (derive, default) = match user_agent {
        None => (quote! { #[derive(Clone, Default)] }, quote! {}),
        Some(product) => {
            let prefix = format!("{product}/");
            let doc =
                format!(" own, its `User-Agent` is `{product}/` and the version of this crate.");
            let default = quote! {
                impl Default for HttpClient {
                    /// A transport with a default `reqwest::Client`. Outside the browser, which sends its
                    #[doc = #doc]
                    fn default() -> Self {
                        let client = Self::new(reqwest::Client::default());
                        #[cfg(not(target_arch = "wasm32"))]
                        let client =
                            client.with_user_agent(concat!(#prefix, env!("CARGO_PKG_VERSION")));
                        client
                    }
                }
            };
            (quote! { #[derive(Clone)] }, default)
        }
    };
    let tokens = quote! {
        use reqwest::{Method, RequestBuilder};
        use url::Url;

        /// The HTTP transport of the generated clients: a `reqwest::Client`, plus a bearer token
        /// and a `User-Agent` that go in every request when set.
        #derive
        pub struct HttpClient {
            client: reqwest::Client,
            token: Option<String>,
            user_agent: Option<String>,
        }

        #default

        // Hand-written to redact the token.
        impl ::std::fmt::Debug for HttpClient {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.debug_struct("HttpClient")
                    .field("token", &self.token.as_ref().map(|_| "<redacted>"))
                    .field("user_agent", &self.user_agent)
                    .finish_non_exhaustive()
            }
        }

        impl HttpClient {
            /// A transport over `client`, without a token or `User-Agent`.
            pub fn new(client: reqwest::Client) -> Self {
                Self { client, token: None, user_agent: None }
            }

            /// The same transport, sending `token` as the bearer token of every request.
            pub fn with_token(mut self, token: impl Into<String>) -> Self {
                self.token = Some(token.into());
                self
            }

            /// The same transport, sending `user_agent` as the `User-Agent` of every request.
            pub fn with_user_agent(mut self, user_agent: impl Into<String>) -> Self {
                self.user_agent = Some(user_agent.into());
                self
            }

            /// Start a `GET` request.
            pub fn get(&self, url: Url) -> RequestBuilder {
                self.request(Method::GET, url)
            }

            /// Start a `POST` request.
            pub fn post(&self, url: Url) -> RequestBuilder {
                self.request(Method::POST, url)
            }

            /// Start a `PUT` request.
            pub fn put(&self, url: Url) -> RequestBuilder {
                self.request(Method::PUT, url)
            }

            /// Start a `PATCH` request.
            pub fn patch(&self, url: Url) -> RequestBuilder {
                self.request(Method::PATCH, url)
            }

            /// Start a `DELETE` request.
            pub fn delete(&self, url: Url) -> RequestBuilder {
                self.request(Method::DELETE, url)
            }

            /// Start a `HEAD` request.
            pub fn head(&self, url: Url) -> RequestBuilder {
                self.request(Method::HEAD, url)
            }

            /// Start a request with any method.
            pub fn request(&self, method: Method, url: Url) -> RequestBuilder {
                let mut builder = self.client.request(method, url);
                if let Some(user_agent) = &self.user_agent {
                    builder = builder.header(reqwest::header::USER_AGENT, user_agent);
                }
                match &self.token {
                    Some(token) => builder.bearer_auth(token),
                    None => builder,
                }
            }
        }
    };
    format_tokens_static(tokens)
}
