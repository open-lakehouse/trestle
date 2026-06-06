// @generated — do not edit by hand.
use olai_http::CloudClient;
use url::Url;
use crate::Result;
use example_common::models::catalog::v1::*;
/// HTTP client for service operations
#[derive(Clone)]
pub struct CatalogClient {
    pub(crate) client: CloudClient,
    pub(crate) base_url: Url,
}
impl CatalogClient {
    /// Create a new client instance
    pub fn new(client: CloudClient, mut base_url: Url) -> Self {
        if !base_url.path().ends_with('/') {
            base_url.set_path(&format!("{}/", base_url.path()));
        }
        Self { client, base_url }
    }
    pub async fn create_catalog(
        &self,
        request: &CreateCatalogRequest,
    ) -> Result<Catalog> {
        let url = self.base_url.join("catalogs")?;
        let response = self.client.post(url).json(request).send().await?;
        if !response.status().is_success() {
            return Err(crate::error::parse_error_response(response).await);
        }
        let result = response.bytes().await?;
        Ok(serde_json::from_slice(&result)?)
    }
    pub async fn get_catalog(&self, request: &GetCatalogRequest) -> Result<Catalog> {
        let formatted_path = format!("catalogs/{}", request.name);
        let url = self.base_url.join(&formatted_path)?;
        let response = self.client.get(url).send().await?;
        if !response.status().is_success() {
            return Err(crate::error::parse_error_response(response).await);
        }
        let result = response.bytes().await?;
        Ok(serde_json::from_slice(&result)?)
    }
    pub async fn list_catalogs(
        &self,
        request: &ListCatalogsRequest,
    ) -> Result<ListCatalogsResponse> {
        let mut url = self.base_url.join("catalogs")?;
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
    pub async fn update_catalog(
        &self,
        request: &UpdateCatalogRequest,
    ) -> Result<Catalog> {
        let formatted_path = format!("catalogs/{}", request.name);
        let url = self.base_url.join(&formatted_path)?;
        let response = self.client.patch(url).json(request).send().await?;
        if !response.status().is_success() {
            return Err(crate::error::parse_error_response(response).await);
        }
        let result = response.bytes().await?;
        Ok(serde_json::from_slice(&result)?)
    }
    pub async fn delete_catalog(
        &self,
        request: &DeleteCatalogRequest,
    ) -> Result<DeleteCatalogResponse> {
        let formatted_path = format!("catalogs/{}", request.name);
        let url = self.base_url.join(&formatted_path)?;
        let response = self.client.delete(url).send().await?;
        if !response.status().is_success() {
            return Err(crate::error::parse_error_response(response).await);
        }
        let result = response.bytes().await?;
        Ok(serde_json::from_slice(&result)?)
    }
    /// Custom POST RPC without path params — covers `RequestType::Custom(Post)`
    /// dispatched as a collection method (the shape used by factory-style
    /// RPCs like `GenerateTemporary*Credentials`).
    pub async fn generate_catalog_token(
        &self,
        request: &GenerateCatalogTokenRequest,
    ) -> Result<CatalogToken> {
        let url = self.base_url.join("catalog-tokens")?;
        let response = self.client.post(url).json(request).send().await?;
        if !response.status().is_success() {
            return Err(crate::error::parse_error_response(response).await);
        }
        let result = response.bytes().await?;
        Ok(serde_json::from_slice(&result)?)
    }
}
