// @generated — do not edit by hand.
#![allow(unused_imports)]
use olai_http::CloudClient;
use url::Url;
use crate::Result;
use example_common::models::tags::v1::*;
/// HTTP client for service operations
#[derive(Clone)]
pub struct TagAssignmentClient {
    pub(crate) client: CloudClient,
    pub(crate) base_url: Url,
}
impl TagAssignmentClient {
    /// Create a new client instance
    pub fn new(client: CloudClient, mut base_url: Url) -> Self {
        if !base_url.path().ends_with('/') {
            base_url.set_path(&format!("{}/", base_url.path()));
        }
        Self { client, base_url }
    }
    /// List assignments for an entity. Path params: entity_type, entity_name.
    pub async fn list_tag_assignments(
        &self,
        request: &ListTagAssignmentsRequest,
    ) -> Result<ListTagAssignmentsResponse> {
        let formatted_path = format!(
            "entities/{}/{}/tags", request.entity_type, request.entity_name
        );
        let mut url = self.base_url.join(&formatted_path)?;
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
    /// Create/assign a tag. Path params: entity_type, entity_name; body: tag.
    pub async fn create_tag_assignment(
        &self,
        request: &CreateTagAssignmentRequest,
    ) -> Result<TagAssignment> {
        let formatted_path = format!(
            "entities/{}/{}/tags", request.entity_type, request.entity_name
        );
        let url = self.base_url.join(&formatted_path)?;
        let response = self.client.post(url).json(request).send().await?;
        if !response.status().is_success() {
            return Err(crate::error::parse_error_response(response).await);
        }
        let result = response.bytes().await?;
        Ok(serde_json::from_slice(&result)?)
    }
    /// Get a single assignment. Composite key: entity_type, entity_name, tag_key.
    /// Carries a gnostic `operation_id` to exercise annotation-driven binding method naming
    /// (the binding method should be named `fetch_tag_assignment`, not `get_tag_assignment`).
    pub async fn get_tag_assignment(
        &self,
        request: &GetTagAssignmentRequest,
    ) -> Result<TagAssignment> {
        let formatted_path = format!(
            "entities/{}/{}/tags/{}", request.entity_type, request.entity_name, request
            .tag_key
        );
        let url = self.base_url.join(&formatted_path)?;
        let response = self.client.get(url).send().await?;
        if !response.status().is_success() {
            return Err(crate::error::parse_error_response(response).await);
        }
        let result = response.bytes().await?;
        Ok(serde_json::from_slice(&result)?)
    }
    /// Delete a single assignment. Composite key path params.
    pub async fn delete_tag_assignment(
        &self,
        request: &DeleteTagAssignmentRequest,
    ) -> Result<DeleteTagAssignmentResponse> {
        let formatted_path = format!(
            "entities/{}/{}/tags/{}", request.entity_type, request.entity_name, request
            .tag_key
        );
        let url = self.base_url.join(&formatted_path)?;
        let response = self.client.delete(url).send().await?;
        if !response.status().is_success() {
            return Err(crate::error::parse_error_response(response).await);
        }
        let result = response.bytes().await?;
        Ok(serde_json::from_slice(&result)?)
    }
    /// Custom POST RPC targeting a composite key that returns `Empty` — exercises
    /// the `<()>` / void-return path for a resource-less, path-param'd method.
    pub async fn touch_tag_assignment(
        &self,
        request: &TouchTagAssignmentRequest,
    ) -> Result<()> {
        let formatted_path = format!(
            "entities/{}/{}/tags/{}:touch", request.entity_type, request.entity_name,
            request.tag_key
        );
        let url = self.base_url.join(&formatted_path)?;
        let response = self.client.post(url).json(request).send().await?;
        if !response.status().is_success() {
            return Err(crate::error::parse_error_response(response).await);
        }
        Ok(())
    }
}
