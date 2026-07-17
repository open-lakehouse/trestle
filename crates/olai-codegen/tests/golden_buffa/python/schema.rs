// @generated — do not edit by hand.
use std::collections::HashMap;
use pyo3::prelude::*;
use example_client::SchemaClient;
use example_common::models::schemas::v1::*;
use example_common::models::*;
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
        view: PyGetSchemaRequestView,
    ) -> PyExampleResult<PySchema> {
        let request = self.client.get(view.into());
        let runtime = get_runtime(py)?;
        py.detach(|| {
            #[allow(clippy::let_unit_value)]
            let result = runtime.block_on(request.into_future())?;
            Ok::<_, PyExampleError>(PySchema::from(result))
        })
    }
    #[pyo3(signature = (schema = None))]
    pub fn update(
        &self,
        py: Python,
        schema: ::core::option::Option<PySchema>,
    ) -> PyExampleResult<PySchema> {
        let mut request = self.client.update();
        request = request.with_schema(schema.map(::core::convert::Into::into));
        let runtime = get_runtime(py)?;
        py.detach(|| {
            #[allow(clippy::let_unit_value)]
            let result = runtime.block_on(request.into_future())?;
            Ok::<_, PyExampleError>(PySchema::from(result))
        })
    }
    pub fn delete(&self, py: Python) -> PyExampleResult<()> {
        let request = self.client.delete();
        let runtime = get_runtime(py)?;
        py.detach(|| {
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
