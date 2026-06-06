// @generated — do not edit by hand.
use olai_http::CloudClient;
use url::Url;
use crate::Result;
use example_common::models::catalog::v1::*;
/// HTTP client for service operations
#[derive(Clone)]
pub struct QueryClient {
    pub(crate) client: CloudClient,
    pub(crate) base_url: Url,
}
impl QueryClient {
    /// Create a new client instance
    pub fn new(client: CloudClient, mut base_url: Url) -> Self {
        if !base_url.path().ends_with('/') {
            base_url.set_path(&format!("{}/", base_url.path()));
        }
        Self { client, base_url }
    }
    /// Repeated string query param
    pub async fn list_by_tags(
        &self,
        request: &ListByTagsRequest,
    ) -> Result<ListByTagsResponse> {
        let mut url = self.base_url.join("schemas")?;
        for value in &request.tags {
            url.query_pairs_mut().append_pair("tags", &value.to_string());
        }
        url.query_pairs_mut()
            .append_pair("max_results", &request.max_results.to_string());
        let response = self.client.get(url).send().await?;
        if !response.status().is_success() {
            return Err(crate::error::parse_error_response(response).await);
        }
        let result = response.bytes().await?;
        Ok(serde_json::from_slice(&result)?)
    }
    /// Enum query param
    pub async fn list_by_catalog_type(
        &self,
        request: &ListByCatalogTypeRequest,
    ) -> Result<ListByTagsResponse> {
        let mut url = self.base_url.join("catalogs/by-type")?;
        url.query_pairs_mut()
            .append_pair("catalog_type", &request.catalog_type.to_string());
        let response = self.client.get(url).send().await?;
        if !response.status().is_success() {
            return Err(crate::error::parse_error_response(response).await);
        }
        let result = response.bytes().await?;
        Ok(serde_json::from_slice(&result)?)
    }
}
