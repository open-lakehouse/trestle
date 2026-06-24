// @generated — do not edit by hand.
use olai_http::CloudClient;
use url::Url;
use crate::api::Result;
use golden_path_app_common::models::golden_path_app::v1::*;
/// HTTP client for service operations
#[derive(Clone)]
pub struct GreetingServiceClient {
    pub(crate) client: CloudClient,
    pub(crate) base_url: Url,
}
impl GreetingServiceClient {
    /// Create a new client instance
    pub fn new(client: CloudClient, mut base_url: Url) -> Self {
        if !base_url.path().ends_with('/') {
            base_url.set_path(&format!("{}/", base_url.path()));
        }
        Self { client, base_url }
    }
    /// Create a new greeting.
    pub async fn create_greeting(
        &self,
        request: &CreateGreetingRequest,
    ) -> Result<Greeting> {
        let url = self.base_url.join("v1/greetings")?;
        let response = self.client.post(url).json(request).send().await?;
        if !response.status().is_success() {
            return Err(crate::error::parse_error_response(response).await);
        }
        let result = response.bytes().await?;
        Ok(serde_json::from_slice(&result)?)
    }
    /// Fetch a greeting by name.
    pub async fn get_greeting(&self, request: &GetGreetingRequest) -> Result<Greeting> {
        let formatted_path = format!("v1/{}", request.name);
        let url = self.base_url.join(&formatted_path)?;
        let response = self.client.get(url).send().await?;
        if !response.status().is_success() {
            return Err(crate::error::parse_error_response(response).await);
        }
        let result = response.bytes().await?;
        Ok(serde_json::from_slice(&result)?)
    }
}
