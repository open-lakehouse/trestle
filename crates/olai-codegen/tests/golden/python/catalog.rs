// @generated — do not edit by hand.
use std::collections::HashMap;
use pyo3::prelude::*;
use example_client::CatalogClient;
use example_common::models::catalog::v1::*;
use example_common::models::*;
use crate::codegen::error::{PyExampleError, PyExampleResult};

/// The requests on one `Catalog`.
#[pyclass(name = "CatalogClient", module = "example._core.catalog")]
pub struct PyCatalogClient {
    pub(crate) client: CatalogClient,
}

#[pymethods]
impl PyCatalogClient {
    /// Returns:
    ///     The requested resource
    pub fn get(&self, py: Python) -> PyExampleResult<PyCatalog> {
        let request = self.client.get();
        #[allow(clippy::let_unit_value)]
        let result = crate::codegen::runtime::block_on(py, request.into_future())??;
        Ok(PyCatalog::from(result))
    }

    /// Returns:
    ///     The requested resource
    #[pyo3(signature = (catalog = None))]
    pub fn update(
        &self,
        py: Python,
        catalog: ::core::option::Option<PyCatalog>,
    ) -> PyExampleResult<PyCatalog> {
        let request = self
            .client
            .update()
            .with_catalog(catalog.map(::core::convert::Into::into));
        #[allow(clippy::let_unit_value)]
        let result = crate::codegen::runtime::block_on(py, request.into_future())??;
        Ok(PyCatalog::from(result))
    }

    pub fn delete(&self, py: Python) -> PyExampleResult<()> {
        let request = self.client.delete();
        #[allow(clippy::let_unit_value)]
        let result = crate::codegen::runtime::block_on(py, request.into_future())??;
        Ok(result)
    }

    /// Resource-targeted custom GET (path param, not a collection method) — exercises surfacing a custom
    /// read on the scoped client (`catalog.get_catalog_status()`) instead of leaving its generated builder
    /// orphaned.
    ///
    /// Returns:
    ///     The requested resource
    pub fn get_catalog_status(&self, py: Python) -> PyExampleResult<PyCatalogStatus> {
        let request = self.client.get_catalog_status();
        #[allow(clippy::let_unit_value)]
        let result = crate::codegen::runtime::block_on(py, request.into_future())??;
        Ok(PyCatalogStatus::from(result))
    }
}

impl PyCatalogClient {
    pub fn new(client: CatalogClient) -> Self {
        Self { client }
    }
}
