// @generated — do not edit by hand.
use std::collections::HashMap;
use pyo3::prelude::*;
use example_client::CatalogClient;
use example_common::models::catalog::v1::*;
use example_common::models::*;
use crate::error::{PyExampleError, PyExampleResult};
use crate::runtime::get_runtime;
#[pyclass(name = "CatalogClient")]
pub struct PyCatalogClient {
    pub(crate) client: CatalogClient,
}
#[pymethods]
impl PyCatalogClient {
    pub fn get(&self, py: Python) -> PyExampleResult<PyCatalog> {
        let request = self.client.get();
        let runtime = get_runtime(py)?;
        py.detach(|| {
            #[allow(clippy::let_unit_value)]
            let result = runtime.block_on(request.into_future())?;
            Ok::<_, PyExampleError>(PyCatalog::from(result))
        })
    }
    #[pyo3(signature = (catalog = None))]
    pub fn update(
        &self,
        py: Python,
        catalog: ::core::option::Option<PyCatalog>,
    ) -> PyExampleResult<PyCatalog> {
        let mut request = self.client.update();
        request = request.with_catalog(catalog.map(::core::convert::Into::into));
        let runtime = get_runtime(py)?;
        py.detach(|| {
            #[allow(clippy::let_unit_value)]
            let result = runtime.block_on(request.into_future())?;
            Ok::<_, PyExampleError>(PyCatalog::from(result))
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
    pub fn get_catalog_status(&self, py: Python) -> PyExampleResult<PyCatalogStatus> {
        let request = self.client.get_catalog_status();
        let runtime = get_runtime(py)?;
        py.detach(|| {
            #[allow(clippy::let_unit_value)]
            let result = runtime.block_on(request.into_future())?;
            Ok::<_, PyExampleError>(PyCatalogStatus::from(result))
        })
    }
}
impl PyCatalogClient {
    pub fn new(client: CatalogClient) -> Self {
        Self { client }
    }
}
