// @generated — do not edit by hand.
use std::collections::HashMap;
use pyo3::prelude::*;
use example_client::CatalogClient;
use example_common::models::catalog::v1::*;
use crate::error::{PyExampleError, PyExampleResult};
use crate::runtime::get_runtime;
#[pyclass(name = "CatalogClient")]
pub struct PyCatalogClient {
    pub(crate) client: CatalogClient,
}
#[pymethods]
impl PyCatalogClient {
    pub fn get(&self, py: Python) -> PyExampleResult<Catalog> {
        let mut request = self.client.get();
        let runtime = get_runtime(py)?;
        py.allow_threads(|| {
            let result = runtime.block_on(request.into_future())?;
            Ok::<_, PyExampleError>(result)
        })
    }
    #[pyo3(signature = (catalog = None))]
    pub fn update(
        &self,
        py: Python,
        catalog: Option<Catalog>,
    ) -> PyExampleResult<Catalog> {
        let mut request = self.client.update();
        request = request.with_catalog(catalog);
        let runtime = get_runtime(py)?;
        py.allow_threads(|| {
            let result = runtime.block_on(request.into_future())?;
            Ok::<_, PyExampleError>(result)
        })
    }
    pub fn delete(&self, py: Python) -> PyExampleResult<()> {
        let mut request = self.client.delete();
        let runtime = get_runtime(py)?;
        py.allow_threads(|| {
            runtime.block_on(request.into_future())?;
            Ok::<_, PyExampleError>(())
        })
    }
}
impl PyCatalogClient {
    pub fn new(client: CatalogClient) -> Self {
        Self { client }
    }
}
