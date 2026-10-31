// @generated — do not edit by hand.
use std::collections::HashMap;
use pyo3::prelude::*;
use example_client::SchemaClient;
use example_common::models::schemas::v1::*;
use example_common::models::*;
use crate::codegen::error::{PyExampleError, PyExampleResult};

/// The requests on one `Schema`.
#[pyclass(name = "SchemaClient", module = "example._core.schemas")]
pub struct PySchemaClient {
    pub(crate) client: SchemaClient,
}

#[pymethods]
impl PySchemaClient {
    /// Returns:
    ///     A Schema is a child resource of a Catalog. This fixture exercises the generated
    ///     nested resource-
    ///     scoped client: `catalog.schema(name)` navigation, `catalog.create_schema(...)`, and
    ///     the cross-
    ///     module model import for the create method's enum parameter (`SchemaType`).
    #[pyo3(signature = (view))]
    pub fn get(
        &self,
        py: Python,
        view: PyGetSchemaRequestView,
    ) -> PyExampleResult<PySchema> {
        let request = self.client.get(view.into());
        let runtime = crate::codegen::runtime::rt();
        py.detach(|| {
            #[allow(clippy::let_unit_value)]
            let result = runtime.block_on(request.into_future())?;
            Ok::<_, PyExampleError>(PySchema::from(result))
        })
    }

    /// Returns:
    ///     A Schema is a child resource of a Catalog. This fixture exercises the generated
    ///     nested resource-
    ///     scoped client: `catalog.schema(name)` navigation, `catalog.create_schema(...)`, and
    ///     the cross-
    ///     module model import for the create method's enum parameter (`SchemaType`).
    #[pyo3(signature = (schema = None))]
    pub fn update(
        &self,
        py: Python,
        schema: ::core::option::Option<PySchema>,
    ) -> PyExampleResult<PySchema> {
        let request = self
            .client
            .update()
            .with_schema(schema.map(::core::convert::Into::into));
        let runtime = crate::codegen::runtime::rt();
        py.detach(|| {
            #[allow(clippy::let_unit_value)]
            let result = runtime.block_on(request.into_future())?;
            Ok::<_, PyExampleError>(PySchema::from(result))
        })
    }

    pub fn delete(&self, py: Python) -> PyExampleResult<()> {
        let request = self.client.delete();
        let runtime = crate::codegen::runtime::rt();
        py.detach(|| {
            #[allow(clippy::let_unit_value)]
            let result = runtime.block_on(request.into_future())?;
            Ok::<_, PyExampleError>(result)
        })
    }
}

impl PySchemaClient {
    pub fn new(client: SchemaClient) -> Self {
        Self { client }
    }
}
