// @generated — do not edit by hand.
use example_common::models::schemas::v1::*;
use super::builders::*;
use super::client::SchemaServiceClient;
/// A client scoped to a single `schema`.
#[derive(Clone)]
pub struct SchemaClient {
    pub(crate) catalog_name: String,
    pub(crate) schema_name: String,
    pub(crate) client: SchemaServiceClient,
}
impl SchemaClient {
    /// Create a client bound to the resource's name components.
    pub fn new(
        catalog_name: impl Into<String>,
        schema_name: impl Into<String>,
        client: SchemaServiceClient,
    ) -> Self {
        Self {
            catalog_name: catalog_name.into(),
            schema_name: schema_name.into(),
            client,
        }
    }
    pub fn get(&self) -> GetSchemaBuilder {
        GetSchemaBuilder::new(
            self.client.clone(),
            format!("{}.{}", self.catalog_name, self.schema_name),
        )
    }
    pub fn update(&self) -> UpdateSchemaBuilder {
        UpdateSchemaBuilder::new(
            self.client.clone(),
            format!("{}.{}", self.catalog_name, self.schema_name),
        )
    }
    pub fn delete(&self) -> DeleteSchemaBuilder {
        DeleteSchemaBuilder::new(
            self.client.clone(),
            format!("{}.{}", self.catalog_name, self.schema_name),
        )
    }
}
