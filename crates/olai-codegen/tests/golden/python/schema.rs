// @generated — do not edit by hand.
use std::collections::HashMap;
use futures::stream::StreamExt;
use pyo3::IntoPyObjectExt;
use pyo3::prelude::*;
use crate::error::PyExampleError;
use crate::codegen::runtime::{block_on, ListIterator};
use crate::codegen::runtime::AsyncListIterator;
use example_client::SchemaClient;
use example_common::models::schemas::v1::*;
use example_common::models::*;
#[pyclass(name = "SchemaClient")]
pub struct PySchemaClient {
    pub(crate) client: SchemaClient,
}
#[pymethods]
impl PySchemaClient {
    #[pyo3(signature = (view))]
    pub fn get(
        &self,
        py: Python<'_>,
        view: PyGetSchemaRequestView,
    ) -> PyResult<PySchema> {
        let request = self.client.get(view.into());
        let result = block_on(py, request.into_future())?.map_err(PyExampleError::from)?;
        Ok(PySchema::from(result))
    }
    #[pyo3(signature = (schema = None))]
    pub fn update(
        &self,
        py: Python<'_>,
        schema: ::core::option::Option<PySchema>,
    ) -> PyResult<PySchema> {
        let mut request = self.client.update();
        request = request.with_schema(schema.map(::core::convert::Into::into));
        let result = block_on(py, request.into_future())?.map_err(PyExampleError::from)?;
        Ok(PySchema::from(result))
    }
    pub fn delete(&self, py: Python<'_>) -> PyResult<()> {
        let request = self.client.delete();
        block_on(py, request.into_future())?.map_err(PyExampleError::from)?;
        Ok(())
    }
}
impl PySchemaClient {
    pub fn new(client: SchemaClient) -> Self {
        Self { client }
    }
}
#[pyclass(name = "AsyncSchemaClient")]
pub struct PyAsyncSchemaClient {
    pub(crate) client: SchemaClient,
}
#[pymethods]
impl PyAsyncSchemaClient {
    #[pyo3(signature = (view))]
    pub fn get<'py>(
        &self,
        py: Python<'py>,
        view: PyGetSchemaRequestView,
    ) -> PyResult<Bound<'py, PyAny>> {
        let request = self.client.get(view.into());
        ::pyo3_async_runtimes::tokio::future_into_py(
            py,
            async move {
                let result = request.await.map_err(PyExampleError::from)?;
                Ok::<_, PyErr>(PySchema::from(result))
            },
        )
    }
    #[pyo3(signature = (schema = None))]
    pub fn update<'py>(
        &self,
        py: Python<'py>,
        schema: ::core::option::Option<PySchema>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let mut request = self.client.update();
        request = request.with_schema(schema.map(::core::convert::Into::into));
        ::pyo3_async_runtimes::tokio::future_into_py(
            py,
            async move {
                let result = request.await.map_err(PyExampleError::from)?;
                Ok::<_, PyErr>(PySchema::from(result))
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
}
impl PyAsyncSchemaClient {
    pub fn new(client: SchemaClient) -> Self {
        Self { client }
    }
}
