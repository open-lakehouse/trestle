// @generated — do not edit by hand.
pub mod catalog;
pub mod schema;
use std::collections::HashMap;
use futures::stream::TryStreamExt;
use pyo3::prelude::*;
use example_client::ExampleClient;
use crate::error::{PyExampleError, PyExampleResult};
use crate::runtime::get_runtime;
use example_common::models::catalog::v1::*;
use example_common::models::catalog::v1::*;
use example_common::models::schemas::v1::*;
use example_common::models::tags::v1::*;
use crate::codegen::catalog::PyCatalogClient;
use crate::codegen::schema::PySchemaClient;
#[pyclass(name = "ExampleClient")]
pub struct PyExampleClient {
    client: ExampleClient,
}
#[pymethods]
impl PyExampleClient {
    #[new]
    #[pyo3(signature = (base_url, token = None))]
    pub fn new(base_url: String, token: Option<String>) -> PyResult<Self> {
        let client = if let Some(token) = token {
            olai_http::CloudClient::new_with_token(token)
        } else {
            olai_http::CloudClient::new_unauthenticated()
        };
        let base_url = base_url.parse().map_err(PyExampleError::from)?;
        Ok(Self {
            client: ExampleClient::new(client, base_url),
        })
    }
    #[pyo3(signature = (catalog = None))]
    pub fn create_catalog(
        &self,
        py: Python,
        catalog: Option<Catalog>,
    ) -> PyExampleResult<Catalog> {
        let mut request = self.client.create_catalog();
        request = request.with_catalog(catalog);
        let runtime = get_runtime(py)?;
        py.allow_threads(|| {
            let result = runtime.block_on(request.into_future())?;
            Ok::<_, PyExampleError>(result)
        })
    }
    #[pyo3(signature = (max_results))]
    pub fn list_catalogs(
        &self,
        py: Python,
        max_results: i32,
    ) -> PyExampleResult<Vec<Catalog>> {
        let mut request = self.client.list_catalogs(max_results, page_token);
        let runtime = get_runtime(py)?;
        py.allow_threads(|| {
            let result = runtime
                .block_on(async move { request.into_stream().try_collect().await })?;
            Ok::<_, PyExampleError>(result)
        })
    }
    #[pyo3(signature = (catalog_id))]
    pub fn generate_catalog_token(
        &self,
        py: Python,
        catalog_id: String,
    ) -> PyExampleResult<CatalogToken> {
        let mut request = self.client.generate_catalog_token(catalog_id);
        let runtime = get_runtime(py)?;
        py.allow_threads(|| {
            let result = runtime.block_on(request.into_future())?;
            Ok::<_, PyExampleError>(result)
        })
    }
    #[pyo3(signature = (tags, max_results))]
    pub fn list_by_tags(
        &self,
        py: Python,
        tags: Option<Vec<String>>,
        max_results: i32,
    ) -> PyExampleResult<ListByTagsResponse> {
        let mut request = self.client.list_by_tags(tags, max_results);
        let runtime = get_runtime(py)?;
        py.allow_threads(|| {
            let result = runtime.block_on(request.into_future())?;
            Ok::<_, PyExampleError>(result)
        })
    }
    #[pyo3(signature = (catalog_type))]
    pub fn list_by_catalog_type(
        &self,
        py: Python,
        catalog_type: CatalogType,
    ) -> PyExampleResult<ListByTagsResponse> {
        let mut request = self.client.list_by_catalog_type(catalog_type);
        let runtime = get_runtime(py)?;
        py.allow_threads(|| {
            let result = runtime.block_on(request.into_future())?;
            Ok::<_, PyExampleError>(result)
        })
    }
    #[pyo3(signature = (name, catalog_name, schema_type))]
    pub fn create_schema(
        &self,
        py: Python,
        name: String,
        catalog_name: String,
        schema_type: SchemaType,
    ) -> PyExampleResult<Schema> {
        let mut request = self.client.create_schema(name, catalog_name, schema_type);
        let runtime = get_runtime(py)?;
        py.allow_threads(|| {
            let result = runtime.block_on(request.into_future())?;
            Ok::<_, PyExampleError>(result)
        })
    }
    #[pyo3(signature = (catalog_name, max_results))]
    pub fn list_schemas(
        &self,
        py: Python,
        catalog_name: String,
        max_results: i32,
    ) -> PyExampleResult<Vec<Schema>> {
        let mut request = self
            .client
            .list_schemas(catalog_name, max_results, page_token);
        let runtime = get_runtime(py)?;
        py.allow_threads(|| {
            let result = runtime
                .block_on(async move { request.into_stream().try_collect().await })?;
            Ok::<_, PyExampleError>(result)
        })
    }
    #[pyo3(signature = (entity_type, entity_name, max_results, page_token))]
    pub fn list_tag_assignments(
        &self,
        py: Python,
        entity_type: String,
        entity_name: String,
        max_results: i32,
        page_token: String,
    ) -> PyExampleResult<ListTagAssignmentsResponse> {
        let mut request = self
            .client
            .list_tag_assignments(entity_type, entity_name, max_results, page_token);
        let runtime = get_runtime(py)?;
        py.allow_threads(|| {
            let result = runtime.block_on(request.into_future())?;
            Ok::<_, PyExampleError>(result)
        })
    }
    #[pyo3(signature = (entity_type, entity_name, tag = None))]
    pub fn create_tag_assignment(
        &self,
        py: Python,
        entity_type: String,
        entity_name: String,
        tag: Option<TagAssignment>,
    ) -> PyExampleResult<TagAssignment> {
        let mut request = self.client.create_tag_assignment(entity_type, entity_name);
        request = request.with_tag(tag);
        let runtime = get_runtime(py)?;
        py.allow_threads(|| {
            let result = runtime.block_on(request.into_future())?;
            Ok::<_, PyExampleError>(result)
        })
    }
    #[pyo3(signature = (entity_type, entity_name, tag_key))]
    pub fn fetch_tag_assignment(
        &self,
        py: Python,
        entity_type: String,
        entity_name: String,
        tag_key: String,
    ) -> PyExampleResult<TagAssignment> {
        let mut request = self
            .client
            .get_tag_assignment(entity_type, entity_name, tag_key);
        let runtime = get_runtime(py)?;
        py.allow_threads(|| {
            let result = runtime.block_on(request.into_future())?;
            Ok::<_, PyExampleError>(result)
        })
    }
    #[pyo3(signature = (entity_type, entity_name, tag_key))]
    pub fn delete_tag_assignment(
        &self,
        py: Python,
        entity_type: String,
        entity_name: String,
        tag_key: String,
    ) -> PyExampleResult<DeleteTagAssignmentResponse> {
        let mut request = self
            .client
            .delete_tag_assignment(entity_type, entity_name, tag_key);
        let runtime = get_runtime(py)?;
        py.allow_threads(|| {
            let result = runtime.block_on(request.into_future())?;
            Ok::<_, PyExampleError>(result)
        })
    }
    #[pyo3(signature = (entity_type, entity_name, tag_key))]
    pub fn touch_tag_assignment(
        &self,
        py: Python,
        entity_type: String,
        entity_name: String,
        tag_key: String,
    ) -> PyExampleResult<()> {
        let mut request = self
            .client
            .touch_tag_assignment(entity_type, entity_name, tag_key);
        let runtime = get_runtime(py)?;
        py.allow_threads(|| {
            let result = runtime.block_on(request.into_future())?;
            Ok::<_, PyExampleError>(result)
        })
    }
    pub fn catalog(&self, catalog_name: String) -> PyCatalogClient {
        PyCatalogClient {
            client: self.client.catalog(catalog_name),
        }
    }
    pub fn schema(&self, catalog_name: String, schema_name: String) -> PySchemaClient {
        let full_name = format!("{}.{}", catalog_name, schema_name);
        PySchemaClient {
            client: self.client.schema_from_full_name(full_name),
        }
    }
}
