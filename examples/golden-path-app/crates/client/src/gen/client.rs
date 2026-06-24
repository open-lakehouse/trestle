// @generated — do not edit by hand.
use crate::codegen::greeting::*;
#[cfg(not(target_arch = "wasm32"))]
use ::olai_http::CloudClient as Transport;
#[cfg(target_arch = "wasm32")]
use ::olai_http_wasm::WasmClient as Transport;
use golden_path_app_common::models::golden_path_app::v1::*;
use url::Url;
#[derive(Clone)]
pub struct GoldenPathAppClient {
    client: Transport,
    base_url: Url,
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
