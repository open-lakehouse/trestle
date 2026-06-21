// @generated — do not edit by hand.
use olai_http::CloudClient;
use url::Url;
use crate::Result;
use example_common::models::schemas::v1::*;
/// HTTP client for service operations
#[derive(Clone)]
pub struct SchemaServiceClient {
    pub(crate) client: CloudClient,
    pub(crate) base_url: Url,
}
impl SchemaServiceClient {
    /// Create a new client instance
    pub fn new(client: CloudClient, mut base_url: Url) -> Self {
        if !base_url.path().ends_with('/') {
            base_url.set_path(&format!("{}/", base_url.path()));
        }
        Self { client, base_url }
    }
    pub async fn create_schema(&self, request: &CreateSchemaRequest) -> Result<Schema> {
        let url = self.base_url.join("schemas")?;
        let response = self.client.post(url).json(request).send().await?;
        if !response.status().is_success() {
            return Err(crate::error::parse_error_response(response).await);
        }
        let result = response.bytes().await?;
        Ok(serde_json::from_slice(&result)?)
    }
    pub async fn get_schema(&self, request: &GetSchemaRequest) -> Result<Schema> {
        let formatted_path = format!("schemas/{}", request.full_name);
        let mut url = self.base_url.join(&formatted_path)?;
        if let Some(known) = request.view.as_known() {
            use buffa::Enumeration as _;
            url.query_pairs_mut().append_pair("view", known.proto_name());
        }
        let response = self.client.get(url).send().await?;
        if !response.status().is_success() {
            return Err(crate::error::parse_error_response(response).await);
        }
        let result = response.bytes().await?;
        Ok(serde_json::from_slice(&result)?)
    }
    pub async fn list_schemas(
        &self,
        request: &ListSchemasRequest,
    ) -> Result<ListSchemasResponse> {
        let mut url = self.base_url.join("schemas")?;
        url.query_pairs_mut().append_pair("catalog_name", &request.catalog_name);
        url.query_pairs_mut()
            .append_pair("max_results", &request.max_results.to_string());
        url.query_pairs_mut().append_pair("page_token", &request.page_token);
        let response = self.client.get(url).send().await?;
        if !response.status().is_success() {
            return Err(crate::error::parse_error_response(response).await);
        }
        let result = response.bytes().await?;
        Ok(serde_json::from_slice(&result)?)
    }
    pub async fn update_schema(&self, request: &UpdateSchemaRequest) -> Result<Schema> {
        let formatted_path = format!("schemas/{}", request.full_name);
        let url = self.base_url.join(&formatted_path)?;
        let response = self.client.patch(url).json(request).send().await?;
        if !response.status().is_success() {
            return Err(crate::error::parse_error_response(response).await);
        }
        let result = response.bytes().await?;
        Ok(serde_json::from_slice(&result)?)
    }
    pub async fn delete_schema(
        &self,
        request: &DeleteSchemaRequest,
    ) -> Result<DeleteSchemaResponse> {
        let formatted_path = format!("schemas/{}", request.full_name);
        let url = self.base_url.join(&formatted_path)?;
        let response = self.client.delete(url).send().await?;
        if !response.status().is_success() {
            return Err(crate::error::parse_error_response(response).await);
        }
        let result = response.bytes().await?;
        Ok(serde_json::from_slice(&result)?)
    }
}
