// @generated — do not edit by hand.
use olai_http::CloudClient;
use url::Url;
use crate::codegen::catalog::*;
use crate::codegen::schema::*;
use crate::codegen::tag_assignments::*;
use example_common::models::catalog::v1::*;
use example_common::models::tags::v1::*;
use crate::CatalogClient;
#[derive(Clone)]
pub struct ExampleClient {
    catalog: crate::codegen::catalog::CatalogClient,
    schema: crate::codegen::schema::SchemaClient,
    tag_assignments: crate::codegen::tag_assignments::TagAssignmentClient,
}
impl ExampleClient {
    /// Create a new aggregate client from a cloud client and base URL.
    pub fn new(client: CloudClient, mut base_url: Url) -> Self {
        if !base_url.path().ends_with('/') {
            base_url.set_path(&format!("{}/", base_url.path()));
        }
        let catalog = crate::codegen::catalog::CatalogClient::new(
            client.clone(),
            base_url.clone(),
        );
        let schema = crate::codegen::schema::SchemaClient::new(
            client.clone(),
            base_url.clone(),
        );
        let tag_assignments = crate::codegen::tag_assignments::TagAssignmentClient::new(
            client.clone(),
            base_url.clone(),
        );
        Self {
            catalog,
            schema,
            tag_assignments,
        }
    }
    /// Create a new aggregate client with no authentication.
    pub fn new_unauthenticated(base_url: Url) -> Self {
        Self::new(CloudClient::new_unauthenticated(), base_url)
    }
    /// Create a new aggregate client authenticating with a bearer token.
    pub fn new_with_token(base_url: Url, token: impl ToString) -> Self {
        Self::new(CloudClient::new_with_token(token), base_url)
    }
    ///Low-level `catalog` client exposing request/response passthrough methods.
    pub fn catalog_client(&self) -> crate::codegen::catalog::CatalogClient {
        self.catalog.clone()
    }
    ///Low-level `schema` client exposing request/response passthrough methods.
    pub fn schema_client(&self) -> crate::codegen::schema::SchemaClient {
        self.schema.clone()
    }
    ///Low-level `tag_assignments` client exposing request/response passthrough methods.
    pub fn tag_assignments_client(
        &self,
    ) -> crate::codegen::tag_assignments::TagAssignmentClient {
        self.tag_assignments.clone()
    }
    pub fn create_catalog(&self) -> CreateCatalogBuilder {
        CreateCatalogBuilder::new(self.catalog.clone())
    }
    pub fn list_catalogs(
        &self,
        max_results: i32,
        page_token: impl Into<String>,
    ) -> ListCatalogsBuilder {
        ListCatalogsBuilder::new(self.catalog.clone(), max_results, page_token)
    }
    pub fn generate_catalog_token(
        &self,
        catalog_id: impl Into<String>,
    ) -> GenerateCatalogTokenBuilder {
        GenerateCatalogTokenBuilder::new(self.catalog.clone(), catalog_id)
    }
    pub fn catalog(&self, catalog_name: impl ToString) -> CatalogClient {
        CatalogClient::new(catalog_name, self.catalog.clone())
    }
    pub fn list_by_tags(
        &self,
        tags: Vec<impl Into<String>>,
        max_results: i32,
    ) -> ListByTagsBuilder {
        ListByTagsBuilder::new(self.schema.clone(), tags, max_results)
    }
    pub fn list_by_catalog_type(
        &self,
        catalog_type: CatalogType,
    ) -> ListByCatalogTypeBuilder {
        ListByCatalogTypeBuilder::new(self.schema.clone(), catalog_type)
    }
    pub fn list_tag_assignments(
        &self,
        entity_type: impl Into<String>,
        entity_name: impl Into<String>,
        max_results: i32,
        page_token: impl Into<String>,
    ) -> ListTagAssignmentsBuilder {
        ListTagAssignmentsBuilder::new(
            self.tag_assignments.clone(),
            entity_type,
            entity_name,
            max_results,
            page_token,
        )
    }
    pub fn create_tag_assignment(
        &self,
        entity_type: impl Into<String>,
        entity_name: impl Into<String>,
    ) -> CreateTagAssignmentBuilder {
        CreateTagAssignmentBuilder::new(
            self.tag_assignments.clone(),
            entity_type,
            entity_name,
        )
    }
    pub fn fetch_tag_assignment(
        &self,
        entity_type: impl Into<String>,
        entity_name: impl Into<String>,
        tag_key: impl Into<String>,
    ) -> GetTagAssignmentBuilder {
        GetTagAssignmentBuilder::new(
            self.tag_assignments.clone(),
            entity_type,
            entity_name,
            tag_key,
        )
    }
    pub fn delete_tag_assignment(
        &self,
        entity_type: impl Into<String>,
        entity_name: impl Into<String>,
        tag_key: impl Into<String>,
    ) -> DeleteTagAssignmentBuilder {
        DeleteTagAssignmentBuilder::new(
            self.tag_assignments.clone(),
            entity_type,
            entity_name,
            tag_key,
        )
    }
    pub fn touch_tag_assignment(
        &self,
        entity_type: impl Into<String>,
        entity_name: impl Into<String>,
        tag_key: impl Into<String>,
    ) -> TouchTagAssignmentBuilder {
        TouchTagAssignmentBuilder::new(
            self.tag_assignments.clone(),
            entity_type,
            entity_name,
            tag_key,
        )
    }
}
