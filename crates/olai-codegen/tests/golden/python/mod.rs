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
    /// A client of the API at `base_url`. `token`, when set, goes in every request as
    /// the bearer token. `token_provider` is asked for the token instead: a
    /// `TokenProvider`, or a callable that returns the token, or a pair of the token
    /// and the time it expires, in seconds since the epoch; a token with its expiry is
    /// kept until a minute before it, a bare token is asked for at every request.
    #[new]
    #[pyo3(signature = (base_url, token = None, token_provider = None))]
    pub fn new(
        py: Python<'_>,
        base_url: String,
        token: Option<String>,
        token_provider: Option<::pyo3::Py<::pyo3::PyAny>>,
    ) -> PyResult<Self> {
        let base_url: ::url::Url = base_url.parse().map_err(PyExampleError::from)?;
        let signer: Option<::std::sync::Arc<dyn ::olai_http::RequestSigner>> = match (
            token,
            token_provider,
        ) {
            (Some(_), Some(_)) => {
                return Err(
                    ::pyo3::exceptions::PyValueError::new_err(
                        "token and token_provider cannot both be set",
                    ),
                );
            }
            (Some(token), None) => {
                Some(
                    ::std::sync::Arc::new(
                        ::olai_http::bearer::BearerProviderSigner(
                            ::olai_http::bearer::BearerAuth::static_token(token),
                        ),
                    ),
                )
            }
            (None, Some(provider)) => {
                Some(
                    match provider.bind(py).cast::<RustTokenProvider>() {
                        Ok(provider) => provider.get().signer.clone(),
                        Err(_) => ::std::sync::Arc::new(PyTokenProvider::new(provider)),
                    },
                )
            }
            (None, None) => None,
        };
        let options = ExampleClient::client_options();
        let retry = ::olai_http::RetryConfig::default();
        let client = match signer {
            Some(signer) => {
                ::olai_http::CloudClient::new_with_request_signer(
                    signer,
                    &options,
                    retry,
                    None,
                )
            }
            None => {
                ::olai_http::CloudClient::new_unauthenticated_with_options(
                    &options,
                    retry,
                    None,
                )
            }
        }
            .map_err(|e: ::olai_http::Error| PyExampleError::from(
                example_client::error::Error::from(e),
            ))?;
        Ok(Self {
            client: ExampleClient::new(client, base_url),
        })
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
        #[allow(clippy::let_unit_value)]
        let result = crate::codegen::runtime::block_on(py, request.into_future())??;
        Ok(PyCatalog::from(result))
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
        #[allow(clippy::let_unit_value)]
        let result = crate::codegen::runtime::block_on(
            py,
            async move {
                request.into_stream().try_collect::<::std::vec::Vec<_>>().await
            },
        )??;
        Ok(result.into_iter().map(PyCatalog::from).collect::<::std::vec::Vec<_>>())
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
        #[allow(clippy::let_unit_value)]
        let result = crate::codegen::runtime::block_on(py, request.into_future())??;
        Ok(PyCatalogToken::from(result))
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
        #[allow(clippy::let_unit_value)]
        let result = crate::codegen::runtime::block_on(py, request.into_future())??;
        Ok(PyListByTagsResponse::from(result))
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
        #[allow(clippy::let_unit_value)]
        let result = crate::codegen::runtime::block_on(py, request.into_future())??;
        Ok(PyListByTagsResponse::from(result))
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
        #[allow(clippy::let_unit_value)]
        let result = crate::codegen::runtime::block_on(py, request.into_future())??;
        Ok(PySchema::from(result))
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
        #[allow(clippy::let_unit_value)]
        let result = crate::codegen::runtime::block_on(
            py,
            async move {
                request.into_stream().try_collect::<::std::vec::Vec<_>>().await
            },
        )??;
        Ok(result.into_iter().map(PySchema::from).collect::<::std::vec::Vec<_>>())
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
        #[allow(clippy::let_unit_value)]
        let result = crate::codegen::runtime::block_on(py, request.into_future())??;
        Ok(PyListTagAssignmentsResponse::from(result))
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
        #[allow(clippy::let_unit_value)]
        let result = crate::codegen::runtime::block_on(py, request.into_future())??;
        Ok(PyTagAssignment::from(result))
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
        #[allow(clippy::let_unit_value)]
        let result = crate::codegen::runtime::block_on(py, request.into_future())??;
        Ok(PyTagAssignment::from(result))
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
        #[allow(clippy::let_unit_value)]
        let result = crate::codegen::runtime::block_on(py, request.into_future())??;
        Ok(PyDeleteTagAssignmentResponse::from(result))
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
        #[allow(clippy::let_unit_value)]
        let result = crate::codegen::runtime::block_on(py, request.into_future())??;
        Ok(result)
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

/// A token provider made in Rust, for `token_provider=`. The binding crate makes one
/// from any `olai_http::RequestSigner`, such as a bearer provider in a
/// `BearerProviderSigner`.
#[pyclass(name = "TokenProvider", frozen, module = "example._core")]
pub struct RustTokenProvider {
    pub(crate) signer: ::std::sync::Arc<dyn ::olai_http::RequestSigner>,
}

impl RustTokenProvider {
    pub fn new(signer: ::std::sync::Arc<dyn ::olai_http::RequestSigner>) -> Self {
        Self { signer }
    }
}

/// The bearer tokens of a `token_provider` callable: the token, or a
/// `(token, expires_at)` pair with the expiry in seconds since the epoch, kept until a
/// minute before it. The callable runs on a blocking thread, one call at a time: it may
/// block, and call the blocking methods of this module.
pub struct PyTokenProvider {
    provider: ::std::sync::Arc<::pyo3::Py<::pyo3::PyAny>>,
    kept: ::futures::lock::Mutex<Option<(String, ::std::time::Instant)>>,
}

impl PyTokenProvider {
    fn new(provider: ::pyo3::Py<::pyo3::PyAny>) -> Self {
        Self {
            provider: ::std::sync::Arc::new(provider),
            kept: ::futures::lock::Mutex::new(None),
        }
    }

    /// Ask the callable: the token, and its expiry when it gives one.
    fn ask(provider: &::pyo3::Py<::pyo3::PyAny>) -> PyResult<(String, Option<f64>)> {
        ::pyo3::Python::attach(|py| {
            let value = provider.bind(py).call0()?;
            if let Ok(token) = value.extract::<String>() {
                return Ok((token, None));
            }
            let (token, expires_at) = value
                .extract::<(String, f64)>()
                .map_err(|_| {
                    ::pyo3::exceptions::PyTypeError::new_err(
                        "token_provider must return the token, or a (token, \
                                 expires_at) pair with the expiry in seconds since the epoch",
                    )
                })?;
            Ok((token, Some(expires_at)))
        })
    }

    /// The token: the kept one, else the callable's.
    async fn token(&self) -> ::olai_http::Result<String> {
        let failed = |source: Box<dyn ::std::error::Error + Send + Sync>| {
            ::olai_http::Error::Generic {
                source,
            }
        };
        let mut kept = self.kept.lock().await;
        if let Some((token, until)) = kept.as_ref()
            && ::std::time::Instant::now() < *until
        {
            return Ok(token.clone());
        }
        let provider = self.provider.clone();
        let (token, expires_at) = ::tokio::task::spawn_blocking(move || Self::ask(
                &provider,
            ))
            .await
            .map_err(|e| failed(Box::new(e)))?
            .map_err(|e| failed(Box::new(e)))?;
        if let Some(expires_at) = expires_at {
            let now = ::std::time::SystemTime::now()
                .duration_since(::std::time::UNIX_EPOCH)
                .map_or(0.0, |d| d.as_secs_f64());
            let left = expires_at - now - 60.0;
            let left = if left.is_finite() { left.clamp(0.0, 1e9) } else { 0.0 };
            let until = ::std::time::Instant::now()
                + ::std::time::Duration::from_secs_f64(left);
            *kept = Some((token.clone(), until));
        }
        Ok(token)
    }
}

impl ::std::fmt::Debug for PyTokenProvider {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        f.debug_struct("PyTokenProvider").finish_non_exhaustive()
    }
}

impl ::olai_http::RequestSigner for PyTokenProvider {
    fn sign<'a>(
        &'a self,
        request: ::olai_http::reqwest::RequestBuilder,
    ) -> ::futures::future::BoxFuture<
        'a,
        ::olai_http::Result<::olai_http::reqwest::RequestBuilder>,
    > {
        Box::pin(async move { Ok(request.bearer_auth(self.token().await?)) })
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
    use super::RustTokenProvider;

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
