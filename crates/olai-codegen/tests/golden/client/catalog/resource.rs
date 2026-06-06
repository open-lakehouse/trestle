// @generated — do not edit by hand.
use example_common::models::catalog::v1::*;
use super::builders::*;
use super::client::CatalogServiceClient;
/// A client scoped to a single `catalog`.
#[derive(Clone)]
pub struct CatalogClient {
    catalog_name: String,
    client: CatalogServiceClient,
}
impl CatalogClient {
    /// Create a client bound to the resource's name components.
    pub fn new(catalog_name: impl Into<String>, client: CatalogServiceClient) -> Self {
        Self {
            catalog_name: catalog_name.into(),
            client,
        }
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
}
