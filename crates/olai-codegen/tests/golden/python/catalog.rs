// @generated — do not edit by hand.
use std::collections::HashMap;
use futures::stream::StreamExt;
use pyo3::IntoPyObjectExt;
use pyo3::prelude::*;
use crate::error::PyExampleError;
use crate::codegen::runtime::{block_on, ListIterator};
use crate::codegen::runtime::AsyncListIterator;
use example_client::CatalogClient;
use example_common::models::catalog::v1::*;
use example_common::models::*;
#[pyclass(name = "CatalogClient")]
pub struct PyCatalogClient {
    pub(crate) client: CatalogClient,
}
#[pymethods]
impl PyCatalogClient {
    pub fn get(&self, py: Python<'_>) -> PyResult<PyCatalog> {
        let request = self.client.get();
        let result = block_on(py, request.into_future())?.map_err(PyExampleError::from)?;
        Ok(PyCatalog::from(result))
    }
    #[pyo3(signature = (catalog = None))]
    pub fn update(
        &self,
        py: Python<'_>,
        catalog: ::core::option::Option<PyCatalog>,
    ) -> PyResult<PyCatalog> {
        let mut request = self.client.update();
        request = request.with_catalog(catalog.map(::core::convert::Into::into));
        let result = block_on(py, request.into_future())?.map_err(PyExampleError::from)?;
        Ok(PyCatalog::from(result))
    }
    pub fn delete(&self, py: Python<'_>) -> PyResult<()> {
        let request = self.client.delete();
        block_on(py, request.into_future())?.map_err(PyExampleError::from)?;
        Ok(())
    }
    pub fn get_catalog_status(&self, py: Python<'_>) -> PyResult<PyCatalogStatus> {
        let request = self.client.get_catalog_status();
        let result = block_on(py, request.into_future())?.map_err(PyExampleError::from)?;
        Ok(PyCatalogStatus::from(result))
    }
}
impl PyCatalogClient {
    pub fn new(client: CatalogClient) -> Self {
        Self { client }
    }
}
#[pyclass(name = "AsyncCatalogClient")]
pub struct PyAsyncCatalogClient {
    pub(crate) client: CatalogClient,
}
#[pymethods]
impl PyAsyncCatalogClient {
    pub fn get<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let request = self.client.get();
        ::pyo3_async_runtimes::tokio::future_into_py(
            py,
            async move {
                let result = request.await.map_err(PyExampleError::from)?;
                Ok::<_, PyErr>(PyCatalog::from(result))
            },
        )
    }
    #[pyo3(signature = (catalog = None))]
    pub fn update<'py>(
        &self,
        py: Python<'py>,
        catalog: ::core::option::Option<PyCatalog>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let mut request = self.client.update();
        request = request.with_catalog(catalog.map(::core::convert::Into::into));
        ::pyo3_async_runtimes::tokio::future_into_py(
            py,
            async move {
                let result = request.await.map_err(PyExampleError::from)?;
                Ok::<_, PyErr>(PyCatalog::from(result))
            },
        )
    }
    pub fn delete<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let request = self.client.delete();
        ::pyo3_async_runtimes::tokio::future_into_py(
            py,
            async move {
                request.await.map_err(PyExampleError::from)?;
                Ok::<_, PyErr>(None::<()>)
            },
        )
    }
    pub fn get_catalog_status<'py>(
        &self,
        py: Python<'py>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let request = self.client.get_catalog_status();
        ::pyo3_async_runtimes::tokio::future_into_py(
            py,
            async move {
                let result = request.await.map_err(PyExampleError::from)?;
                Ok::<_, PyErr>(PyCatalogStatus::from(result))
            },
        )
    }
}
impl PyAsyncCatalogClient {
    pub fn new(client: CatalogClient) -> Self {
        Self { client }
    }
}
