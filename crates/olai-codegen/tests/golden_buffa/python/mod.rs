// @generated — do not edit by hand.
#![allow(dead_code, unused_imports, clippy::too_many_arguments)]

#[allow(dead_code, unused_imports, clippy::too_many_arguments)]
pub mod catalog;
#[allow(dead_code, unused_imports, clippy::too_many_arguments)]
pub mod schema;
pub mod error;
pub mod runtime;
use std::collections::HashMap;
use futures::stream::TryStreamExt;
use pyo3::prelude::*;
use example_client::ExampleClient;
use crate::codegen::error::{PyExampleError, PyExampleResult};
use example_common::models::catalog::v1::*;
use example_common::models::schemas::v1::*;
use example_common::models::tags::v1::*;
use example_common::models::*;
use crate::codegen::catalog::PyCatalogClient;
use crate::codegen::schema::PySchemaClient;

/// A client of the API at one base URL.
#[pyclass(name = "ExampleClient", module = "example._core")]
pub struct PyExampleClient {
    pub(crate) client: ExampleClient,
}

#[pymethods]
impl PyExampleClient {
    /// A client of the API at `base_url`. `token`, when set, goes in every
    /// request as the bearer token.
    #[new]
    #[pyo3(signature = (base_url, token = None))]
    pub fn new(base_url: String, token: Option<String>) -> PyResult<Self> {
        let base_url = base_url.parse().map_err(PyExampleError::from)?;
        let client = match token {
            Some(token) => ExampleClient::new_with_token(base_url, token),
            None => ExampleClient::new_unauthenticated(base_url),
        };
        Ok(Self { client })
    }

    /// Returns:
    ///     The requested resource
    #[pyo3(signature = (catalog = None))]
    pub fn create_catalog(
        &self,
        py: Python,
        catalog: ::core::option::Option<PyCatalog>,
    ) -> PyExampleResult<PyCatalog> {
        let request = self
            .client
            .create_catalog()
            .with_catalog(catalog.map(::core::convert::Into::into));
        let runtime = crate::codegen::runtime::rt();
        py.detach(|| {
            #[allow(clippy::let_unit_value)]
            let result = runtime.block_on(request.into_future())?;
            Ok::<_, PyExampleError>(PyCatalog::from(result))
        })
    }

    /// Returns:
    ///     The items.
    #[pyo3(signature = (max_results))]
    pub fn list_catalogs(
        &self,
        py: Python,
        max_results: i32,
    ) -> PyExampleResult<::std::vec::Vec<PyCatalog>> {
        let request = self
            .client
            .list_catalogs(max_results, ::std::string::String::new());
        let runtime = crate::codegen::runtime::rt();
        py.detach(|| {
            #[allow(clippy::let_unit_value)]
            let result = runtime
                .block_on(async move {
                    request.into_stream().try_collect::<::std::vec::Vec<_>>().await
                })?;
            Ok::<
                _,
                PyExampleError,
            >(result.into_iter().map(PyCatalog::from).collect::<::std::vec::Vec<_>>())
        })
    }

    /// Custom POST RPC without path params — covers `RequestType::Custom(Post)` dispatched as a collection
    /// method (the shape used by factory-style RPCs like `GenerateTemporary*Credentials`).
    ///
    /// Returns:
    ///     The requested resource
    #[pyo3(signature = (catalog_id))]
    pub fn generate_catalog_token(
        &self,
        py: Python,
        catalog_id: String,
    ) -> PyExampleResult<PyCatalogToken> {
        let request = self.client.generate_catalog_token(catalog_id);
        let runtime = crate::codegen::runtime::rt();
        py.detach(|| {
            #[allow(clippy::let_unit_value)]
            let result = runtime.block_on(request.into_future())?;
            Ok::<_, PyExampleError>(PyCatalogToken::from(result))
        })
    }

    /// Repeated string query param
    ///
    /// Args:
    ///     tags: becomes repeated query param
    ///
    /// Returns:
    ///     The requested resource
    #[pyo3(signature = (tags, max_results))]
    pub fn list_by_tags(
        &self,
        py: Python,
        tags: Vec<String>,
        max_results: i32,
    ) -> PyExampleResult<PyListByTagsResponse> {
        let request = self.client.list_by_tags(tags, max_results);
        let runtime = crate::codegen::runtime::rt();
        py.detach(|| {
            #[allow(clippy::let_unit_value)]
            let result = runtime.block_on(request.into_future())?;
            Ok::<_, PyExampleError>(PyListByTagsResponse::from(result))
        })
    }

    /// Enum query param
    ///
    /// Args:
    ///     catalog_type: enum as query param
    ///
    /// Returns:
    ///     The requested resource
    #[pyo3(signature = (catalog_type))]
    pub fn list_by_catalog_type(
        &self,
        py: Python,
        catalog_type: PyCatalogType,
    ) -> PyExampleResult<PyListByTagsResponse> {
        let request = self.client.list_by_catalog_type(catalog_type.into());
        let runtime = crate::codegen::runtime::rt();
        py.detach(|| {
            #[allow(clippy::let_unit_value)]
            let result = runtime.block_on(request.into_future())?;
            Ok::<_, PyExampleError>(PyListByTagsResponse::from(result))
        })
    }

    /// Args:
    ///     name: Schema's own name (the new component supplied by the caller).
    ///     catalog_name: Parent catalog name — filled from the parent `CatalogClient`'s captured component.
    ///     schema_type: Required enum parameter whose type lives in this (schemas) models module —
    ///                  exercises the child-model import on the parent's generated `create_schema` method.
    ///
    /// Returns:
    ///     A Schema is a child resource of a Catalog. This fixture exercises the generated
    ///     nested resource-
    ///     scoped client: `catalog.schema(name)` navigation, `catalog.create_schema(...)`, and
    ///     the cross-
    ///     module model import for the create method's enum parameter (`SchemaType`).
    #[pyo3(signature = (name, catalog_name, schema_type))]
    pub fn create_schema(
        &self,
        py: Python,
        name: String,
        catalog_name: String,
        schema_type: PySchemaType,
    ) -> PyExampleResult<PySchema> {
        let request = self.client.create_schema(name, catalog_name, schema_type.into());
        let runtime = crate::codegen::runtime::rt();
        py.detach(|| {
            #[allow(clippy::let_unit_value)]
            let result = runtime.block_on(request.into_future())?;
            Ok::<_, PyExampleError>(PySchema::from(result))
        })
    }

    /// Args:
    ///     catalog_name: Parent scoping field carrying the child-type reference that makes Schema a child
    ///                   of Catalog.
    ///
    /// Returns:
    ///     The items.
    #[pyo3(signature = (catalog_name, max_results))]
    pub fn list_schemas(
        &self,
        py: Python,
        catalog_name: String,
        max_results: i32,
    ) -> PyExampleResult<::std::vec::Vec<PySchema>> {
        let request = self
            .client
            .list_schemas(catalog_name, max_results, ::std::string::String::new());
        let runtime = crate::codegen::runtime::rt();
        py.detach(|| {
            #[allow(clippy::let_unit_value)]
            let result = runtime
                .block_on(async move {
                    request.into_stream().try_collect::<::std::vec::Vec<_>>().await
                })?;
            Ok::<
                _,
                PyExampleError,
            >(result.into_iter().map(PySchema::from).collect::<::std::vec::Vec<_>>())
        })
    }

    /// List assignments for an entity. Path params: entity_type, entity_name.
    ///
    /// Returns:
    ///     The requested resource
    #[pyo3(signature = (entity_type, entity_name, max_results, page_token))]
    pub fn list_tag_assignments(
        &self,
        py: Python,
        entity_type: String,
        entity_name: String,
        max_results: i32,
        page_token: String,
    ) -> PyExampleResult<PyListTagAssignmentsResponse> {
        let request = self
            .client
            .list_tag_assignments(entity_type, entity_name, max_results, page_token);
        let runtime = crate::codegen::runtime::rt();
        py.detach(|| {
            #[allow(clippy::let_unit_value)]
            let result = runtime.block_on(request.into_future())?;
            Ok::<_, PyExampleError>(PyListTagAssignmentsResponse::from(result))
        })
    }

    /// Create/assign a tag. Path params: entity_type, entity_name; body: tag.
    ///
    /// Returns:
    ///     The requested resource
    #[pyo3(signature = (entity_type, entity_name, tag = None))]
    pub fn create_tag_assignment(
        &self,
        py: Python,
        entity_type: String,
        entity_name: String,
        tag: ::core::option::Option<PyTagAssignment>,
    ) -> PyExampleResult<PyTagAssignment> {
        let request = self
            .client
            .create_tag_assignment(entity_type, entity_name)
            .with_tag(tag.map(::core::convert::Into::into));
        let runtime = crate::codegen::runtime::rt();
        py.detach(|| {
            #[allow(clippy::let_unit_value)]
            let result = runtime.block_on(request.into_future())?;
            Ok::<_, PyExampleError>(PyTagAssignment::from(result))
        })
    }

    /// Get a single assignment. Composite key: entity_type, entity_name, tag_key. Carries a gnostic
    /// `operation_id` to exercise annotation-driven binding method naming (the binding method should be
    /// named `fetch_tag_assignment`, not `get_tag_assignment`).
    ///
    /// Returns:
    ///     The requested resource
    #[pyo3(signature = (entity_type, entity_name, tag_key))]
    pub fn fetch_tag_assignment(
        &self,
        py: Python,
        entity_type: String,
        entity_name: String,
        tag_key: String,
    ) -> PyExampleResult<PyTagAssignment> {
        let request = self.client.get_tag_assignment(entity_type, entity_name, tag_key);
        let runtime = crate::codegen::runtime::rt();
        py.detach(|| {
            #[allow(clippy::let_unit_value)]
            let result = runtime.block_on(request.into_future())?;
            Ok::<_, PyExampleError>(PyTagAssignment::from(result))
        })
    }

    /// Delete a single assignment. Composite key path params.
    ///
    /// Returns:
    ///     The requested resource
    #[pyo3(signature = (entity_type, entity_name, tag_key))]
    pub fn delete_tag_assignment(
        &self,
        py: Python,
        entity_type: String,
        entity_name: String,
        tag_key: String,
    ) -> PyExampleResult<PyDeleteTagAssignmentResponse> {
        let request = self
            .client
            .delete_tag_assignment(entity_type, entity_name, tag_key);
        let runtime = crate::codegen::runtime::rt();
        py.detach(|| {
            #[allow(clippy::let_unit_value)]
            let result = runtime.block_on(request.into_future())?;
            Ok::<_, PyExampleError>(PyDeleteTagAssignmentResponse::from(result))
        })
    }

    /// Custom POST RPC targeting a composite key that returns `Empty` — exercises the `<()>` / void-return
    /// path for a resource-less, path-param'd method.
    #[pyo3(signature = (entity_type, entity_name, tag_key))]
    pub fn touch_tag_assignment(
        &self,
        py: Python,
        entity_type: String,
        entity_name: String,
        tag_key: String,
    ) -> PyExampleResult<()> {
        let request = self
            .client
            .touch_tag_assignment(entity_type, entity_name, tag_key);
        let runtime = crate::codegen::runtime::rt();
        py.detach(|| {
            #[allow(clippy::let_unit_value)]
            let result = runtime.block_on(request.into_future())?;
            Ok::<_, PyExampleError>(result)
        })
    }

    /// The requests on one `Catalog`. It sends no request itself.
    pub fn catalog(&self, catalog_name: String) -> PyCatalogClient {
        PyCatalogClient {
            client: self.client.catalog(catalog_name),
        }
    }

    /// The requests on one `Schema`. It sends no request itself.
    pub fn schema(&self, catalog_name: String, schema_name: String) -> PySchemaClient {
        let full_name = format!("{}.{}", catalog_name, schema_name);
        PySchemaClient {
            client: self.client.schema_from_full_name(full_name),
        }
    }
}

/// The extension module, as Python imports it.
pub const CORE_MODULE: &str = "example._core";
/// The submodules of the extension module.
const SUBMODULES: &[&str] = &["catalog", "schemas", "tags"];
/// The extension module `example._core`.
#[::pyo3::pymodule]
#[pyo3(name = "_core", module = "example")]
pub mod extension_module {
    use ::pyo3::prelude::*;

    /// The version of the bindings crate.
    #[pymodule_export]
    #[allow(non_upper_case_globals)]
    const __version__: &str = env!("CARGO_PKG_VERSION");
    #[pymodule_export]
    use super::PyExampleClient;

    #[pymodule_export]
    use crate::codegen::error::{
        ExampleError, BadRequestError, UnauthenticatedError, PermissionDeniedError,
        NotFoundError, ConflictError, RateLimitError, ServerError,
    };

    /// The classes of the `catalog` API.
    #[pymodule]
    pub mod catalog {
        #[pymodule_export]
        use super::super::{
            PyAzureConfig, PyCatalog, PyCatalogStatus, PyCatalogToken,
            PyDeleteCatalogResponse, PyListByTagsResponse, PyListCatalogsResponse,
            PyS3Config, PyStorageConfig, PyCatalogClient,
        };
    }

    /// The classes of the `schemas` API.
    #[pymodule]
    pub mod schemas {
        #[pymodule_export]
        use super::super::{
            PyDeleteSchemaResponse, PyListSchemasResponse, PySchema, PySchemaClient,
        };
    }

    /// The classes of the `tags` API.
    #[pymodule]
    pub mod tags {
        #[pymodule_export]
        use super::super::{
            PyDeleteTagAssignmentResponse, PyListTagAssignmentsResponse, PyTagAssignment,
        };
    }

    /// Put the submodules in `sys.modules`. The import system puts the module there
    /// before it runs this. Not handed the module, so the stubs still describe the
    /// module in full.
    #[pymodule_init]
    fn init(py: Python<'_>) -> PyResult<()> {
        let modules = py.import("sys")?.getattr("modules")?;
        match modules.get_item(super::CORE_MODULE) {
            Ok(module) => super::add_submodules_to_sys_modules(module.cast()?),
            Err(_) => Ok(()),
        }
    }
}

/// Put the submodules of the extension module `module` in `sys.modules`, so that
/// `import <module>.<submodule>` finds them. The module does this on import; call it
/// for a module made otherwise, such as with `wrap_pymodule!`.
pub fn add_submodules_to_sys_modules(
    module: &::pyo3::Bound<'_, ::pyo3::types::PyModule>,
) -> ::pyo3::PyResult<()> {
    let modules = module.py().import("sys")?.getattr("modules")?;
    let name = module.name()?;
    for submodule in SUBMODULES {
        modules.set_item(format!("{name}.{submodule}"), module.getattr(*submodule)?)?;
    }
    Ok(())
}
