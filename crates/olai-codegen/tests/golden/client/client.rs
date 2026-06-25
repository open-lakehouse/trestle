// @generated — do not edit by hand.
#![allow(unused_imports)]
use olai_http::CloudClient;
use url::Url;
use crate::codegen::catalog::*;
use crate::codegen::query::*;
use crate::codegen::schema::*;
use crate::codegen::tag_assignments::*;
use example_common::models::catalog::v1::*;
use example_common::models::schemas::v1::*;
use example_common::models::tags::v1::*;
#[derive(Clone)]
pub struct ExampleClient {
    client: CloudClient,
    base_url: Url,
}
impl ExampleClient {
    /// Create a new aggregate client from a cloud client and base URL.
    ///
    /// Per-service clients are constructed on demand (they only hold a cheaply-cloneable
    /// `CloudClient` + `Url`), so nothing is allocated per service here.
    pub fn new(client: CloudClient, mut base_url: Url) -> Self {
        if !base_url.path().ends_with('/') {
            base_url.set_path(&format!("{}/", base_url.path()));
        }
        Self { client, base_url }
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
    pub fn catalog_client(&self) -> crate::codegen::catalog::CatalogServiceClient {
        crate::codegen::catalog::CatalogServiceClient::new(
            self.client.clone(),
            self.base_url.clone(),
        )
    }
    ///Low-level `query` client exposing request/response passthrough methods.
    pub fn query_client(&self) -> crate::codegen::query::QueryClient {
        crate::codegen::query::QueryClient::new(
            self.client.clone(),
            self.base_url.clone(),
        )
    }
    ///Low-level `schema` client exposing request/response passthrough methods.
    pub fn schema_client(&self) -> crate::codegen::schema::SchemaServiceClient {
        crate::codegen::schema::SchemaServiceClient::new(
            self.client.clone(),
            self.base_url.clone(),
        )
    }
    ///Low-level `tag_assignments` client exposing request/response passthrough methods.
    pub fn tag_assignments_client(
        &self,
    ) -> crate::codegen::tag_assignments::TagAssignmentClient {
        crate::codegen::tag_assignments::TagAssignmentClient::new(
            self.client.clone(),
            self.base_url.clone(),
        )
    }
    pub fn create_catalog(&self) -> CreateCatalogBuilder {
        CreateCatalogBuilder::new(
            crate::codegen::catalog::CatalogServiceClient::new(
                self.client.clone(),
                self.base_url.clone(),
            ),
        )
    }
    pub fn list_catalogs(
        &self,
        max_results: i32,
        page_token: impl Into<String>,
    ) -> ListCatalogsBuilder {
        ListCatalogsBuilder::new(
            crate::codegen::catalog::CatalogServiceClient::new(
                self.client.clone(),
                self.base_url.clone(),
            ),
            max_results,
            page_token,
        )
    }
    /// Custom POST RPC without path params — covers `RequestType::Custom(Post)`
    /// dispatched as a collection method (the shape used by factory-style
    /// RPCs like `GenerateTemporary*Credentials`).
    pub fn generate_catalog_token(
        &self,
        catalog_id: impl Into<String>,
    ) -> GenerateCatalogTokenBuilder {
        GenerateCatalogTokenBuilder::new(
            crate::codegen::catalog::CatalogServiceClient::new(
                self.client.clone(),
                self.base_url.clone(),
            ),
            catalog_id,
        )
    }
    /// Access the `catalog` resource scoped to the given name.
    pub fn catalog(&self, catalog_name: impl Into<String>) -> CatalogClient {
        CatalogClient::new(
            catalog_name,
            crate::codegen::catalog::CatalogServiceClient::new(
                self.client.clone(),
                self.base_url.clone(),
            ),
        )
    }
    /// Repeated string query param
    ///
    /// # Arguments
    ///
    /// * `tags` - becomes repeated query param
    pub fn list_by_tags(
        &self,
        tags: Vec<impl Into<String>>,
        max_results: i32,
    ) -> ListByTagsBuilder {
        ListByTagsBuilder::new(
            crate::codegen::query::QueryClient::new(
                self.client.clone(),
                self.base_url.clone(),
            ),
            tags,
            max_results,
        )
    }
    /// Enum query param
    ///
    /// # Arguments
    ///
    /// * `catalog_type` - enum as query param
    pub fn list_by_catalog_type(
        &self,
        catalog_type: CatalogType,
    ) -> ListByCatalogTypeBuilder {
        ListByCatalogTypeBuilder::new(
            crate::codegen::query::QueryClient::new(
                self.client.clone(),
                self.base_url.clone(),
            ),
            catalog_type,
        )
    }
    /// # Arguments
    ///
    /// * `name` - Schema's own name (the new component supplied by the caller).
    /// * `catalog_name` - Parent catalog name — filled from the parent `CatalogClient`'s captured component.
    /// * `schema_type` - Required enum parameter whose type lives in this (schemas) models module — exercises the
    /// child-model import on the parent's generated `create_schema` method.
    pub fn create_schema(
        &self,
        name: impl Into<String>,
        catalog_name: impl Into<String>,
        schema_type: SchemaType,
    ) -> CreateSchemaBuilder {
        CreateSchemaBuilder::new(
            crate::codegen::schema::SchemaServiceClient::new(
                self.client.clone(),
                self.base_url.clone(),
            ),
            name,
            catalog_name,
            schema_type,
        )
    }
    /// # Arguments
    ///
    /// * `catalog_name` - Parent scoping field carrying the child-type reference that makes Schema a child of Catalog.
    pub fn list_schemas(
        &self,
        catalog_name: impl Into<String>,
        max_results: i32,
        page_token: impl Into<String>,
    ) -> ListSchemasBuilder {
        ListSchemasBuilder::new(
            crate::codegen::schema::SchemaServiceClient::new(
                self.client.clone(),
                self.base_url.clone(),
            ),
            catalog_name,
            max_results,
            page_token,
        )
    }
    /// Access the `schema` resource scoped to the given name.
    pub fn schema(
        &self,
        catalog_name: impl Into<String>,
        schema_name: impl Into<String>,
    ) -> SchemaClient {
        SchemaClient::new(
            catalog_name,
            schema_name,
            crate::codegen::schema::SchemaServiceClient::new(
                self.client.clone(),
                self.base_url.clone(),
            ),
        )
    }
    /// Access the `schema` resource from its dot-joined full name.
    pub fn schema_from_full_name(&self, full_name: impl Into<String>) -> SchemaClient {
        SchemaClient::from_full_name(
            full_name,
            crate::codegen::schema::SchemaServiceClient::new(
                self.client.clone(),
                self.base_url.clone(),
            ),
        )
    }
    /// List assignments for an entity. Path params: entity_type, entity_name.
    pub fn list_tag_assignments(
        &self,
        entity_type: impl Into<String>,
        entity_name: impl Into<String>,
        max_results: i32,
        page_token: impl Into<String>,
    ) -> ListTagAssignmentsBuilder {
        ListTagAssignmentsBuilder::new(
            crate::codegen::tag_assignments::TagAssignmentClient::new(
                self.client.clone(),
                self.base_url.clone(),
            ),
            entity_type,
            entity_name,
            max_results,
            page_token,
        )
    }
    /// Create/assign a tag. Path params: entity_type, entity_name; body: tag.
    pub fn create_tag_assignment(
        &self,
        entity_type: impl Into<String>,
        entity_name: impl Into<String>,
    ) -> CreateTagAssignmentBuilder {
        CreateTagAssignmentBuilder::new(
            crate::codegen::tag_assignments::TagAssignmentClient::new(
                self.client.clone(),
                self.base_url.clone(),
            ),
            entity_type,
            entity_name,
        )
    }
    /// Get a single assignment. Composite key: entity_type, entity_name, tag_key.
    /// Carries a gnostic `operation_id` to exercise annotation-driven binding method naming
    /// (the binding method should be named `fetch_tag_assignment`, not `get_tag_assignment`).
    pub fn fetch_tag_assignment(
        &self,
        entity_type: impl Into<String>,
        entity_name: impl Into<String>,
        tag_key: impl Into<String>,
    ) -> GetTagAssignmentBuilder {
        GetTagAssignmentBuilder::new(
            crate::codegen::tag_assignments::TagAssignmentClient::new(
                self.client.clone(),
                self.base_url.clone(),
            ),
            entity_type,
            entity_name,
            tag_key,
        )
    }
    /// Delete a single assignment. Composite key path params.
    pub fn delete_tag_assignment(
        &self,
        entity_type: impl Into<String>,
        entity_name: impl Into<String>,
        tag_key: impl Into<String>,
    ) -> DeleteTagAssignmentBuilder {
        DeleteTagAssignmentBuilder::new(
            crate::codegen::tag_assignments::TagAssignmentClient::new(
                self.client.clone(),
                self.base_url.clone(),
            ),
            entity_type,
            entity_name,
            tag_key,
        )
    }
    /// Custom POST RPC targeting a composite key that returns `Empty` — exercises
    /// the `<()>` / void-return path for a resource-less, path-param'd method.
    pub fn touch_tag_assignment(
        &self,
        entity_type: impl Into<String>,
        entity_name: impl Into<String>,
        tag_key: impl Into<String>,
    ) -> TouchTagAssignmentBuilder {
        TouchTagAssignmentBuilder::new(
            crate::codegen::tag_assignments::TagAssignmentClient::new(
                self.client.clone(),
                self.base_url.clone(),
            ),
            entity_type,
            entity_name,
            tag_key,
        )
    }
}
