// @generated — do not edit by hand.
use std::collections::HashMap;
use pyo3::prelude::*;
use example_client::SchemaClient;
use example_common::models::schemas::v1::*;
use crate::error::{PyExampleError, PyExampleResult};
use crate::runtime::get_runtime;
#[pyclass(name = "SchemaClient")]
pub struct PySchemaClient {
    pub(crate) client: SchemaClient,
}
#[pymethods]
impl PySchemaClient {
    #[pyo3(signature = (view))]
    pub fn get(
        &self,
        py: Python,
        view: get_schema_request::View,
    ) -> PyExampleResult<Schema> {
        let request = self.client.get(view);
        let runtime = get_runtime(py)?;
        py.allow_threads(|| {
            Ok::<_, PyExampleError>(runtime.block_on(request.into_future())?)
        })
    }
    #[pyo3(signature = (schema = None))]
    pub fn update(&self, py: Python, schema: Option<Schema>) -> PyExampleResult<Schema> {
        let mut request = self.client.update();
        request = request.with_schema(schema);
        let runtime = get_runtime(py)?;
        py.allow_threads(|| {
            Ok::<_, PyExampleError>(runtime.block_on(request.into_future())?)
        })
    }
    pub fn delete(&self, py: Python) -> PyExampleResult<()> {
        let request = self.client.delete();
        let runtime = get_runtime(py)?;
        py.allow_threads(|| {
            runtime.block_on(request.into_future())?;
            Ok::<_, PyExampleError>(())
        })
    }
}
impl PySchemaClient {
    pub fn new(client: SchemaClient) -> Self {
        Self { client }
    }
}
