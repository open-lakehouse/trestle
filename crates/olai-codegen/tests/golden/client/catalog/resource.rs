// @generated — do not edit by hand.
use example_common::models::catalog::v1::*;
use example_common::models::schemas::v1::*;
use super::builders::*;
use super::client::CatalogServiceClient;
/// A client scoped to a single `catalog`.
#[derive(Clone)]
pub struct CatalogClient {
    pub(crate) catalog_name: String,
    pub(crate) client: CatalogServiceClient,
}
impl CatalogClient {
    /// Create a client bound to the resource's name components.
    pub fn new(catalog_name: impl Into<String>, client: CatalogServiceClient) -> Self {
        Self {
            catalog_name: catalog_name.into(),
            client,
        }
    }
    /// This resource's own name (the leaf component).
    pub fn name(&self) -> &str {
        &self.catalog_name
    }
    /// The fully-qualified name of this resource.
    pub fn full_name(&self) -> String {
        self.catalog_name.clone()
    }
    pub fn get(&self) -> GetCatalogBuilder {
        GetCatalogBuilder::new(self.client.clone(), &self.catalog_name)
    }
    pub fn update(&self) -> UpdateCatalogBuilder {
        UpdateCatalogBuilder::new(self.client.clone(), &self.catalog_name)
    }
    pub fn delete(&self) -> DeleteCatalogBuilder {
        DeleteCatalogBuilder::new(self.client.clone(), &self.catalog_name)
    }
    /// Resource-targeted custom GET (path param, not a collection method) — exercises surfacing a
    /// custom read on the scoped client (`catalog.get_catalog_status()`) instead of leaving its
    /// generated builder orphaned.
    pub fn get_catalog_status(&self) -> GetCatalogStatusBuilder {
        GetCatalogStatusBuilder::new(self.client.clone(), &self.catalog_name)
    }
    /// Access a `schema` within this resource.
    pub fn schema(
        &self,
        schema_name: impl Into<String>,
    ) -> crate::codegen::schema::SchemaClient {
        crate::codegen::schema::SchemaClient::new(
            &self.catalog_name,
            schema_name,
            crate::codegen::schema::SchemaServiceClient::new(
                self.client.client.clone(),
                self.client.base_url.clone(),
            ),
        )
    }
    /// Create a `schema` within this resource.
    pub fn create_schema(
        &self,
        name: impl Into<String>,
        schema_type: SchemaType,
    ) -> crate::codegen::schema::CreateSchemaBuilder {
        crate::codegen::schema::CreateSchemaBuilder::new(
            crate::codegen::schema::SchemaServiceClient::new(
                self.client.client.clone(),
                self.client.base_url.clone(),
            ),
            name,
            &self.catalog_name,
            schema_type,
        )
    }
    /// List `schema` resources within this resource.
    pub fn list_schemas(
        &self,
        max_results: i32,
        page_token: impl Into<String>,
    ) -> crate::codegen::schema::ListSchemasBuilder {
        crate::codegen::schema::ListSchemasBuilder::new(
            crate::codegen::schema::SchemaServiceClient::new(
                self.client.client.clone(),
                self.client.base_url.clone(),
            ),
            &self.catalog_name,
            max_results,
            page_token,
        )
    }
}
