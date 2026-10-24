// @generated — do not edit by hand.
#![allow(dead_code, unused_imports, clippy::too_many_arguments)]
pub mod runtime;
#[allow(dead_code, unused_imports, clippy::too_many_arguments)]
pub mod catalog;
#[allow(dead_code, unused_imports, clippy::too_many_arguments)]
pub mod schema;
use std::collections::HashMap;
use futures::stream::StreamExt;
use pyo3::IntoPyObjectExt;
use pyo3::prelude::*;
use crate::error::PyExampleError;
use crate::codegen::runtime::{block_on, ListIterator};
use crate::codegen::runtime::AsyncListIterator;
use example_client::ExampleClient;
use example_common::models::catalog::v1::*;
use example_common::models::catalog::v1::*;
use example_common::models::schemas::v1::*;
use example_common::models::tags::v1::*;
use example_common::models::*;
use crate::codegen::catalog::PyCatalogClient;
use crate::codegen::catalog::PyAsyncCatalogClient;
use crate::codegen::schema::PySchemaClient;
use crate::codegen::schema::PyAsyncSchemaClient;
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
        py: Python<'_>,
        catalog: ::core::option::Option<PyCatalog>,
    ) -> PyResult<PyCatalog> {
        let mut request = self.client.create_catalog();
        request = request.with_catalog(catalog.map(::core::convert::Into::into));
        let result = block_on(py, request.into_future())?.map_err(PyExampleError::from)?;
        Ok(PyCatalog::from(result))
    }
    #[pyo3(signature = (max_results))]
    pub fn list_catalogs(&self, max_results: i32) -> PyResult<ListIterator> {
        let request = self
            .client
            .list_catalogs(max_results, ::std::string::String::new());
        let items = request
            .into_stream()
            .map(|item| {
                let item = item.map_err(PyExampleError::from)?;
                Python::attach(|py| PyCatalog::from(item).into_py_any(py))
            });
        Ok(ListIterator::new(items))
    }
    #[pyo3(signature = (catalog_id))]
    pub fn generate_catalog_token(
        &self,
        py: Python<'_>,
        catalog_id: String,
    ) -> PyResult<PyCatalogToken> {
        let request = self.client.generate_catalog_token(catalog_id);
        let result = block_on(py, request.into_future())?.map_err(PyExampleError::from)?;
        Ok(PyCatalogToken::from(result))
    }
    #[pyo3(signature = (tags, max_results))]
    pub fn list_by_tags(
        &self,
        py: Python<'_>,
        tags: Option<Vec<String>>,
        max_results: i32,
    ) -> PyResult<PyListByTagsResponse> {
        let request = self.client.list_by_tags(tags, max_results);
        let result = block_on(py, request.into_future())?.map_err(PyExampleError::from)?;
        Ok(PyListByTagsResponse::from(result))
    }
    #[pyo3(signature = (catalog_type))]
    pub fn list_by_catalog_type(
        &self,
        py: Python<'_>,
        catalog_type: PyCatalogType,
    ) -> PyResult<PyListByTagsResponse> {
        let request = self.client.list_by_catalog_type(catalog_type.into());
        let result = block_on(py, request.into_future())?.map_err(PyExampleError::from)?;
        Ok(PyListByTagsResponse::from(result))
    }
    #[pyo3(signature = (name, catalog_name, schema_type))]
    pub fn create_schema(
        &self,
        py: Python<'_>,
        name: String,
        catalog_name: String,
        schema_type: PySchemaType,
    ) -> PyResult<PySchema> {
        let request = self.client.create_schema(name, catalog_name, schema_type.into());
        let result = block_on(py, request.into_future())?.map_err(PyExampleError::from)?;
        Ok(PySchema::from(result))
    }
    #[pyo3(signature = (catalog_name, max_results))]
    pub fn list_schemas(
        &self,
        catalog_name: String,
        max_results: i32,
    ) -> PyResult<ListIterator> {
        let request = self
            .client
            .list_schemas(catalog_name, max_results, ::std::string::String::new());
        let items = request
            .into_stream()
            .map(|item| {
                let item = item.map_err(PyExampleError::from)?;
                Python::attach(|py| PySchema::from(item).into_py_any(py))
            });
        Ok(ListIterator::new(items))
    }
    #[pyo3(signature = (entity_type, entity_name, max_results, page_token))]
    pub fn list_tag_assignments(
        &self,
        py: Python<'_>,
        entity_type: String,
        entity_name: String,
        max_results: i32,
        page_token: String,
    ) -> PyResult<PyListTagAssignmentsResponse> {
        let request = self
            .client
            .list_tag_assignments(entity_type, entity_name, max_results, page_token);
        let result = block_on(py, request.into_future())?.map_err(PyExampleError::from)?;
        Ok(PyListTagAssignmentsResponse::from(result))
    }
    #[pyo3(signature = (entity_type, entity_name, tag = None))]
    pub fn create_tag_assignment(
        &self,
        py: Python<'_>,
        entity_type: String,
        entity_name: String,
        tag: ::core::option::Option<PyTagAssignment>,
    ) -> PyResult<PyTagAssignment> {
        let mut request = self.client.create_tag_assignment(entity_type, entity_name);
        request = request.with_tag(tag.map(::core::convert::Into::into));
        let result = block_on(py, request.into_future())?.map_err(PyExampleError::from)?;
        Ok(PyTagAssignment::from(result))
    }
    #[pyo3(signature = (entity_type, entity_name, tag_key))]
    pub fn fetch_tag_assignment(
        &self,
        py: Python<'_>,
        entity_type: String,
        entity_name: String,
        tag_key: String,
    ) -> PyResult<PyTagAssignment> {
        let request = self.client.get_tag_assignment(entity_type, entity_name, tag_key);
        let result = block_on(py, request.into_future())?.map_err(PyExampleError::from)?;
        Ok(PyTagAssignment::from(result))
    }
    #[pyo3(signature = (entity_type, entity_name, tag_key))]
    pub fn delete_tag_assignment(
        &self,
        py: Python<'_>,
        entity_type: String,
        entity_name: String,
        tag_key: String,
    ) -> PyResult<PyDeleteTagAssignmentResponse> {
        let request = self
            .client
            .delete_tag_assignment(entity_type, entity_name, tag_key);
        let result = block_on(py, request.into_future())?.map_err(PyExampleError::from)?;
        Ok(PyDeleteTagAssignmentResponse::from(result))
    }
    #[pyo3(signature = (entity_type, entity_name, tag_key))]
    pub fn touch_tag_assignment(
        &self,
        py: Python<'_>,
        entity_type: String,
        entity_name: String,
        tag_key: String,
    ) -> PyResult<()> {
        let request = self
            .client
            .touch_tag_assignment(entity_type, entity_name, tag_key);
        block_on(py, request.into_future())?.map_err(PyExampleError::from)?;
        Ok(())
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
#[pyclass(name = "AsyncExampleClient")]
pub struct PyAsyncExampleClient {
    client: ExampleClient,
}
#[pymethods]
impl PyAsyncExampleClient {
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
    pub fn create_catalog<'py>(
        &self,
        py: Python<'py>,
        catalog: ::core::option::Option<PyCatalog>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let mut request = self.client.create_catalog();
        request = request.with_catalog(catalog.map(::core::convert::Into::into));
        ::pyo3_async_runtimes::tokio::future_into_py(
            py,
            async move {
                let result = request.await.map_err(PyExampleError::from)?;
                Ok::<_, PyErr>(PyCatalog::from(result))
            },
        )
    }
    #[pyo3(signature = (max_results))]
    pub fn list_catalogs(&self, max_results: i32) -> PyResult<AsyncListIterator> {
        let request = self
            .client
            .list_catalogs(max_results, ::std::string::String::new());
        let items = request
            .into_stream()
            .map(|item| {
                let item = item.map_err(PyExampleError::from)?;
                Python::attach(|py| PyCatalog::from(item).into_py_any(py))
            });
        Ok(AsyncListIterator::new(items))
    }
    #[pyo3(signature = (catalog_id))]
    pub fn generate_catalog_token<'py>(
        &self,
        py: Python<'py>,
        catalog_id: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let request = self.client.generate_catalog_token(catalog_id);
        ::pyo3_async_runtimes::tokio::future_into_py(
            py,
            async move {
                let result = request.await.map_err(PyExampleError::from)?;
                Ok::<_, PyErr>(PyCatalogToken::from(result))
            },
        )
    }
    #[pyo3(signature = (tags, max_results))]
    pub fn list_by_tags<'py>(
        &self,
        py: Python<'py>,
        tags: Option<Vec<String>>,
        max_results: i32,
    ) -> PyResult<Bound<'py, PyAny>> {
        let request = self.client.list_by_tags(tags, max_results);
        ::pyo3_async_runtimes::tokio::future_into_py(
            py,
            async move {
                let result = request.await.map_err(PyExampleError::from)?;
                Ok::<_, PyErr>(PyListByTagsResponse::from(result))
            },
        )
    }
    #[pyo3(signature = (catalog_type))]
    pub fn list_by_catalog_type<'py>(
        &self,
        py: Python<'py>,
        catalog_type: PyCatalogType,
    ) -> PyResult<Bound<'py, PyAny>> {
        let request = self.client.list_by_catalog_type(catalog_type.into());
        ::pyo3_async_runtimes::tokio::future_into_py(
            py,
            async move {
                let result = request.await.map_err(PyExampleError::from)?;
                Ok::<_, PyErr>(PyListByTagsResponse::from(result))
            },
        )
    }
    #[pyo3(signature = (name, catalog_name, schema_type))]
    pub fn create_schema<'py>(
        &self,
        py: Python<'py>,
        name: String,
        catalog_name: String,
        schema_type: PySchemaType,
    ) -> PyResult<Bound<'py, PyAny>> {
        let request = self.client.create_schema(name, catalog_name, schema_type.into());
        ::pyo3_async_runtimes::tokio::future_into_py(
            py,
            async move {
                let result = request.await.map_err(PyExampleError::from)?;
                Ok::<_, PyErr>(PySchema::from(result))
            },
        )
    }
    #[pyo3(signature = (catalog_name, max_results))]
    pub fn list_schemas(
        &self,
        catalog_name: String,
        max_results: i32,
    ) -> PyResult<AsyncListIterator> {
        let request = self
            .client
            .list_schemas(catalog_name, max_results, ::std::string::String::new());
        let items = request
            .into_stream()
            .map(|item| {
                let item = item.map_err(PyExampleError::from)?;
                Python::attach(|py| PySchema::from(item).into_py_any(py))
            });
        Ok(AsyncListIterator::new(items))
    }
    #[pyo3(signature = (entity_type, entity_name, max_results, page_token))]
    pub fn list_tag_assignments<'py>(
        &self,
        py: Python<'py>,
        entity_type: String,
        entity_name: String,
        max_results: i32,
        page_token: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let request = self
            .client
            .list_tag_assignments(entity_type, entity_name, max_results, page_token);
        ::pyo3_async_runtimes::tokio::future_into_py(
            py,
            async move {
                let result = request.await.map_err(PyExampleError::from)?;
                Ok::<_, PyErr>(PyListTagAssignmentsResponse::from(result))
            },
        )
    }
    #[pyo3(signature = (entity_type, entity_name, tag = None))]
    pub fn create_tag_assignment<'py>(
        &self,
        py: Python<'py>,
        entity_type: String,
        entity_name: String,
        tag: ::core::option::Option<PyTagAssignment>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let mut request = self.client.create_tag_assignment(entity_type, entity_name);
        request = request.with_tag(tag.map(::core::convert::Into::into));
        ::pyo3_async_runtimes::tokio::future_into_py(
            py,
            async move {
                let result = request.await.map_err(PyExampleError::from)?;
                Ok::<_, PyErr>(PyTagAssignment::from(result))
            },
        )
    }
    #[pyo3(signature = (entity_type, entity_name, tag_key))]
    pub fn fetch_tag_assignment<'py>(
        &self,
        py: Python<'py>,
        entity_type: String,
        entity_name: String,
        tag_key: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let request = self.client.get_tag_assignment(entity_type, entity_name, tag_key);
        ::pyo3_async_runtimes::tokio::future_into_py(
            py,
            async move {
                let result = request.await.map_err(PyExampleError::from)?;
                Ok::<_, PyErr>(PyTagAssignment::from(result))
            },
        )
    }
    #[pyo3(signature = (entity_type, entity_name, tag_key))]
    pub fn delete_tag_assignment<'py>(
        &self,
        py: Python<'py>,
        entity_type: String,
        entity_name: String,
        tag_key: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let request = self
            .client
            .delete_tag_assignment(entity_type, entity_name, tag_key);
        ::pyo3_async_runtimes::tokio::future_into_py(
            py,
            async move {
                let result = request.await.map_err(PyExampleError::from)?;
                Ok::<_, PyErr>(PyDeleteTagAssignmentResponse::from(result))
            },
        )
    }
    #[pyo3(signature = (entity_type, entity_name, tag_key))]
    pub fn touch_tag_assignment<'py>(
        &self,
        py: Python<'py>,
        entity_type: String,
        entity_name: String,
        tag_key: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let request = self
            .client
            .touch_tag_assignment(entity_type, entity_name, tag_key);
        ::pyo3_async_runtimes::tokio::future_into_py(
            py,
            async move {
                request.await.map_err(PyExampleError::from)?;
                Ok::<_, PyErr>(None::<()>)
            },
        )
    }
    pub fn catalog(&self, catalog_name: String) -> PyAsyncCatalogClient {
        PyAsyncCatalogClient {
            client: self.client.catalog(catalog_name),
        }
    }
    pub fn schema(
        &self,
        catalog_name: String,
        schema_name: String,
    ) -> PyAsyncSchemaClient {
        let full_name = format!("{}.{}", catalog_name, schema_name);
        PyAsyncSchemaClient {
            client: self.client.schema_from_full_name(full_name),
        }
    }
}
/// Add every generated client class (and the list pagers) to the extension module `m`.
///
/// Call this from the crate's `#[pymodule]` so newly generated clients are exposed
/// without hand-maintaining an `add_class` list.
pub fn register_clients(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyExampleClient>()?;
    m.add_class::<PyCatalogClient>()?;
    m.add_class::<PySchemaClient>()?;
    m.add_class::<PyAsyncExampleClient>()?;
    m.add_class::<PyAsyncCatalogClient>()?;
    m.add_class::<PyAsyncSchemaClient>()?;
    m.add_class::<ListIterator>()?;
    m.add_class::<AsyncListIterator>()?;
    Ok(())
}
