// @generated — do not edit by hand.
#![allow(unused_imports)]

#[cfg(not(target_arch = "wasm32"))]
use ::olai_http::CloudClient as Transport;

use crate::codegen::greeting::*;
#[cfg(target_arch = "wasm32")]
use ::olai_http_wasm::WasmClient as Transport;
use golden_path_app_common::models::golden_path_app::v1::*;
use url::Url;

#[derive(Clone)]
pub struct GoldenPathAppClient {
    pub(crate) client: Transport,
    pub(crate) base_url: Url,
}

impl GoldenPathAppClient {
    /// Create a new aggregate client from a cloud client and base URL.
    ///
    /// Per-service clients are constructed on demand (they only hold a cheaply-cloneable
    /// `CloudClient` + `Url`), so nothing is allocated per service here.
    pub fn new(client: Transport, mut base_url: Url) -> Self {
        if !base_url.path().ends_with('/') {
            base_url.set_path(&format!("{}/", base_url.path()));
        }
        Self { client, base_url }
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Create a new aggregate client with no authentication.
    pub fn new_unauthenticated(base_url: Url) -> Self {
        Self::new(Transport::new_unauthenticated(), base_url)
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Create a new aggregate client authenticating with a bearer token.
    pub fn new_with_token(base_url: Url, token: impl ToString) -> Self {
        Self::new(Transport::new_with_token(token), base_url)
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// The HTTP settings of the constructors that make the `CloudClient`: plain
    /// `http` URLs are allowed, and no timeout ends a request.
    pub fn client_options() -> ::olai_http::ClientOptions {
        ::olai_http::ClientOptions::new()
            .with_allow_http(true)
            .with_timeout_disabled()
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Create a new aggregate client whose bearer token comes from `provider`, such as
    /// one of [`olai_http::bearer::BearerAuth`], which keeps and refreshes its tokens.
    pub fn new_with_bearer(
        base_url: Url,
        provider: ::olai_http::bearer::BearerCredentialProvider,
    ) -> ::olai_http::Result<Self> {
        let client = Transport::new_with_bearer(
            provider,
            &Self::client_options(),
            ::olai_http::RetryConfig::default(),
            None,
        )?;
        Ok(Self::new(client, base_url))
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Create a new aggregate client that signs its requests with `signer`, for
    /// authentication that a bearer provider does not cover.
    pub fn new_with_signer(
        base_url: Url,
        signer: ::std::sync::Arc<dyn ::olai_http::RequestSigner>,
    ) -> ::olai_http::Result<Self> {
        let client = Transport::new_with_request_signer(
            signer,
            &Self::client_options(),
            ::olai_http::RetryConfig::default(),
            None,
        )?;
        Ok(Self::new(client, base_url))
    }

    #[cfg(target_arch = "wasm32")]
    /// Create a new aggregate client. The browser supplies the session
    /// (cookies / forwarded auth) on each request.
    pub fn new_in_browser(base_url: Url) -> Self {
        Self::new(Transport::new(), base_url)
    }

    ///Low-level `greeting` client exposing request/response passthrough methods.
    pub fn greeting_client(&self) -> crate::codegen::greeting::GreetingServiceClient {
        crate::codegen::greeting::GreetingServiceClient::new(
            self.client.clone(),
            self.base_url.clone(),
        )
    }

    /// Create a new greeting.
    pub fn create_greeting(&self, greeting: Greeting) -> CreateGreetingBuilder {
        CreateGreetingBuilder::new(
            crate::codegen::greeting::GreetingServiceClient::new(
                self.client.clone(),
                self.base_url.clone(),
            ),
            greeting,
        )
    }
}
