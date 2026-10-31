// @generated — do not edit by hand.
/// A member of `example._enums.CatalogType`, or its name.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PyCatalogType(pub super::catalog::v1::CatalogType);
impl PyCatalogType {
    /// The name of the value, or `None` for a value without a Python member.
    fn __name(&self) -> ::core::option::Option<&'static str> {
        match self.0 as i32 {
            1i32 => ::core::option::Option::Some("MANAGED_CATALOG"),
            2i32 => ::core::option::Option::Some("DELTASHARING_CATALOG"),
            _ => ::core::option::Option::None,
        }
    }
}

impl ::core::convert::From<super::catalog::v1::CatalogType> for PyCatalogType {
    fn from(value: super::catalog::v1::CatalogType) -> Self {
        Self(value)
    }
}

impl ::core::convert::From<PyCatalogType> for super::catalog::v1::CatalogType {
    fn from(value: PyCatalogType) -> Self {
        value.0
    }
}

impl<'a, 'py> ::pyo3::FromPyObject<'a, 'py> for PyCatalogType {
    type Error = ::pyo3::PyErr;
    const INPUT_TYPE: ::pyo3::inspect::PyStaticExpr = {
        use ::pyo3::type_hint_union;
        type_hint_union!(
            ::pyo3::type_hint_identifier!("example._enums", "CatalogType"),
            ::pyo3::type_hint_identifier!("builtins", "str")
        )
    };

    fn extract(obj: ::pyo3::Borrowed<'a, 'py, ::pyo3::PyAny>) -> ::pyo3::PyResult<Self> {
        let name: ::pyo3::pybacked::PyBackedStr = ::pyo3::types::PyAnyMethods::extract(
            &*obj,
        )?;
        let n: i32 = match &*name {
            "MANAGED_CATALOG" => 1i32,
            "DELTASHARING_CATALOG" => 2i32,
            other => {
                return ::core::result::Result::Err(
                    ::pyo3::exceptions::PyValueError::new_err(
                        ::std::format!(
                            "invalid CatalogType: {:?}; expected one of: MANAGED_CATALOG, DELTASHARING_CATALOG",
                            other
                        ),
                    ),
                );
            }
        };
        ::core::result::Result::Ok(
            Self(
                <super::catalog::v1::CatalogType as ::core::convert::TryFrom<
                    i32,
                >>::try_from(n)
                    .unwrap_or_default(),
            ),
        )
    }
}

impl<'py> ::pyo3::IntoPyObject<'py> for PyCatalogType {
    type Target = ::pyo3::PyAny;
    type Output = ::pyo3::Bound<'py, ::pyo3::PyAny>;
    type Error = ::pyo3::PyErr;
    const OUTPUT_TYPE: ::pyo3::inspect::PyStaticExpr = ::pyo3::type_hint_identifier!(
        "example._enums", "CatalogType"
    );
    fn into_pyobject(self, py: ::pyo3::Python<'py>) -> ::pyo3::PyResult<Self::Output> {
        static CLASS: ::pyo3::sync::PyOnceLock<::pyo3::Py<::pyo3::types::PyType>> = ::pyo3::sync::PyOnceLock::new();
        let class = CLASS.import(py, "example._enums", "CatalogType")?;
        let name = self
            .__name()
            .ok_or_else(|| {
                ::pyo3::exceptions::PyValueError::new_err(
                    ::std::format!(
                        "CatalogType value {} has no Python member", self.0 as i32
                    ),
                )
            })?;
        ::pyo3::types::PyAnyMethods::getattr(class.as_any(), name)
    }
}

/// A member of `example._enums.GetSchemaRequestView`, or its name.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PyGetSchemaRequestView(pub super::schemas::v1::get_schema_request::View);
impl PyGetSchemaRequestView {
    /// The name of the value, or `None` for a value without a Python member.
    fn __name(&self) -> ::core::option::Option<&'static str> {
        match self.0 as i32 {
            1i32 => ::core::option::Option::Some("BASIC"),
            2i32 => ::core::option::Option::Some("FULL"),
            _ => ::core::option::Option::None,
        }
    }
}

impl ::core::convert::From<super::schemas::v1::get_schema_request::View>
for PyGetSchemaRequestView {
    fn from(value: super::schemas::v1::get_schema_request::View) -> Self {
        Self(value)
    }
}

impl ::core::convert::From<PyGetSchemaRequestView>
for super::schemas::v1::get_schema_request::View {
    fn from(value: PyGetSchemaRequestView) -> Self {
        value.0
    }
}

impl<'a, 'py> ::pyo3::FromPyObject<'a, 'py> for PyGetSchemaRequestView {
    type Error = ::pyo3::PyErr;
    const INPUT_TYPE: ::pyo3::inspect::PyStaticExpr = {
        use ::pyo3::type_hint_union;
        type_hint_union!(
            ::pyo3::type_hint_identifier!("example._enums", "GetSchemaRequestView"),
            ::pyo3::type_hint_identifier!("builtins", "str")
        )
    };

    fn extract(obj: ::pyo3::Borrowed<'a, 'py, ::pyo3::PyAny>) -> ::pyo3::PyResult<Self> {
        let name: ::pyo3::pybacked::PyBackedStr = ::pyo3::types::PyAnyMethods::extract(
            &*obj,
        )?;
        let n: i32 = match &*name {
            "BASIC" => 1i32,
            "FULL" => 2i32,
            other => {
                return ::core::result::Result::Err(
                    ::pyo3::exceptions::PyValueError::new_err(
                        ::std::format!(
                            "invalid GetSchemaRequestView: {:?}; expected one of: BASIC, FULL",
                            other
                        ),
                    ),
                );
            }
        };
        ::core::result::Result::Ok(
            Self(
                <super::schemas::v1::get_schema_request::View as ::core::convert::TryFrom<
                    i32,
                >>::try_from(n)
                    .unwrap_or_default(),
            ),
        )
    }
}

impl<'py> ::pyo3::IntoPyObject<'py> for PyGetSchemaRequestView {
    type Target = ::pyo3::PyAny;
    type Output = ::pyo3::Bound<'py, ::pyo3::PyAny>;
    type Error = ::pyo3::PyErr;
    const OUTPUT_TYPE: ::pyo3::inspect::PyStaticExpr = ::pyo3::type_hint_identifier!(
        "example._enums", "GetSchemaRequestView"
    );
    fn into_pyobject(self, py: ::pyo3::Python<'py>) -> ::pyo3::PyResult<Self::Output> {
        static CLASS: ::pyo3::sync::PyOnceLock<::pyo3::Py<::pyo3::types::PyType>> = ::pyo3::sync::PyOnceLock::new();
        let class = CLASS.import(py, "example._enums", "GetSchemaRequestView")?;
        let name = self
            .__name()
            .ok_or_else(|| {
                ::pyo3::exceptions::PyValueError::new_err(
                    ::std::format!(
                        "GetSchemaRequestView value {} has no Python member", self.0 as
                        i32
                    ),
                )
            })?;
        ::pyo3::types::PyAnyMethods::getattr(class.as_any(), name)
    }
}

/// A member of `example._enums.SchemaType`, or its name.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PySchemaType(pub super::schemas::v1::SchemaType);
impl PySchemaType {
    /// The name of the value, or `None` for a value without a Python member.
    fn __name(&self) -> ::core::option::Option<&'static str> {
        match self.0 as i32 {
            1i32 => ::core::option::Option::Some("MANAGED_SCHEMA"),
            2i32 => ::core::option::Option::Some("EXTERNAL_SCHEMA"),
            _ => ::core::option::Option::None,
        }
    }
}

impl ::core::convert::From<super::schemas::v1::SchemaType> for PySchemaType {
    fn from(value: super::schemas::v1::SchemaType) -> Self {
        Self(value)
    }
}

impl ::core::convert::From<PySchemaType> for super::schemas::v1::SchemaType {
    fn from(value: PySchemaType) -> Self {
        value.0
    }
}

impl<'a, 'py> ::pyo3::FromPyObject<'a, 'py> for PySchemaType {
    type Error = ::pyo3::PyErr;
    const INPUT_TYPE: ::pyo3::inspect::PyStaticExpr = {
        use ::pyo3::type_hint_union;
        type_hint_union!(
            ::pyo3::type_hint_identifier!("example._enums", "SchemaType"),
            ::pyo3::type_hint_identifier!("builtins", "str")
        )
    };

    fn extract(obj: ::pyo3::Borrowed<'a, 'py, ::pyo3::PyAny>) -> ::pyo3::PyResult<Self> {
        let name: ::pyo3::pybacked::PyBackedStr = ::pyo3::types::PyAnyMethods::extract(
            &*obj,
        )?;
        let n: i32 = match &*name {
            "MANAGED_SCHEMA" => 1i32,
            "EXTERNAL_SCHEMA" => 2i32,
            other => {
                return ::core::result::Result::Err(
                    ::pyo3::exceptions::PyValueError::new_err(
                        ::std::format!(
                            "invalid SchemaType: {:?}; expected one of: MANAGED_SCHEMA, EXTERNAL_SCHEMA",
                            other
                        ),
                    ),
                );
            }
        };
        ::core::result::Result::Ok(
            Self(
                <super::schemas::v1::SchemaType as ::core::convert::TryFrom<
                    i32,
                >>::try_from(n)
                    .unwrap_or_default(),
            ),
        )
    }
}

impl<'py> ::pyo3::IntoPyObject<'py> for PySchemaType {
    type Target = ::pyo3::PyAny;
    type Output = ::pyo3::Bound<'py, ::pyo3::PyAny>;
    type Error = ::pyo3::PyErr;
    const OUTPUT_TYPE: ::pyo3::inspect::PyStaticExpr = ::pyo3::type_hint_identifier!(
        "example._enums", "SchemaType"
    );
    fn into_pyobject(self, py: ::pyo3::Python<'py>) -> ::pyo3::PyResult<Self::Output> {
        static CLASS: ::pyo3::sync::PyOnceLock<::pyo3::Py<::pyo3::types::PyType>> = ::pyo3::sync::PyOnceLock::new();
        let class = CLASS.import(py, "example._enums", "SchemaType")?;
        let name = self
            .__name()
            .ok_or_else(|| {
                ::pyo3::exceptions::PyValueError::new_err(
                    ::std::format!(
                        "SchemaType value {} has no Python member", self.0 as i32
                    ),
                )
            })?;
        ::pyo3::types::PyAnyMethods::getattr(class.as_any(), name)
    }
}

#[::pyo3::pyclass(
    name = "AzureConfig",
    eq,
    frozen,
    module = "example._core.catalog",
    from_py_object
)]
#[derive(Clone, Debug, PartialEq)]
pub struct PyAzureConfig(pub super::catalog::v1::AzureConfig);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyAzureConfig {
    #[new]
    #[pyo3(signature = (container = None))]
    fn new(container: ::core::option::Option<::std::string::String>) -> Self {
        let mut inner = <super::catalog::v1::AzureConfig as ::core::default::Default>::default();
        if let ::core::option::Option::Some(value) = container {
            inner.container = value;
        }
        Self(inner)
    }

    #[getter]
    fn container(&self) -> ::std::string::String {
        self.0.container.clone()
    }

    fn __repr__(
        slf: &::pyo3::Bound<'_, Self>,
    ) -> ::pyo3::PyResult<::std::string::String> {
        __python_repr(slf.as_any(), &[("container", false)])
    }
}

impl ::core::convert::From<super::catalog::v1::AzureConfig> for PyAzureConfig {
    fn from(value: super::catalog::v1::AzureConfig) -> Self {
        Self(value)
    }
}

impl ::core::convert::From<PyAzureConfig> for super::catalog::v1::AzureConfig {
    fn from(value: PyAzureConfig) -> Self {
        value.0
    }
}

#[::pyo3::pyclass(
    name = "Catalog",
    eq,
    frozen,
    module = "example._core.catalog",
    from_py_object
)]
#[derive(Clone, Debug, PartialEq)]
pub struct PyCatalog(pub super::catalog::v1::Catalog);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyCatalog {
    #[new]
    #[pyo3(
        signature = (
            name = None,
            comment = None,
            catalog_type = None,
            properties = None,
            storage_config = None,
            created_at = None
        )
    )]
    fn new(
        name: ::core::option::Option<::std::string::String>,
        comment: ::core::option::Option<::std::string::String>,
        catalog_type: ::core::option::Option<PyCatalogType>,
        properties: ::core::option::Option<
            ::std::collections::HashMap<::std::string::String, ::std::string::String>,
        >,
        storage_config: ::core::option::Option<PyStorageConfig>,
        created_at: ::core::option::Option<i64>,
    ) -> Self {
        let mut inner = <super::catalog::v1::Catalog as ::core::default::Default>::default();
        if let ::core::option::Option::Some(value) = name {
            inner.name = value;
        }
        if let ::core::option::Option::Some(value) = comment {
            inner.comment = value;
        }
        {
            let value = catalog_type;
            inner.catalog_type = value
                .map(|e| {
                    <super::catalog::v1::CatalogType as ::core::convert::From<
                        _,
                    >>::from(e) as i32
                })
                .unwrap_or(0);
        }
        if let ::core::option::Option::Some(value) = properties {
            inner.properties = value;
        }
        {
            let value = storage_config;
            inner.storage_config = value.map(|w| w.into());
        }
        if let ::core::option::Option::Some(value) = created_at {
            inner.created_at = value;
        }
        Self(inner)
    }

    #[getter]
    fn name(&self) -> ::std::string::String {
        self.0.name.clone()
    }

    #[getter]
    fn comment(&self) -> ::std::string::String {
        self.0.comment.clone()
    }

    #[getter]
    fn catalog_type(&self) -> ::core::option::Option<PyCatalogType> {
        <super::catalog::v1::CatalogType as ::core::convert::TryFrom<
            i32,
        >>::try_from(self.0.catalog_type)
            .ok()
            .filter(|e| *e as i32 != 0)
            .map(PyCatalogType::from)
    }

    #[getter]
    fn properties(
        &self,
    ) -> ::std::collections::HashMap<::std::string::String, ::std::string::String> {
        self.0.properties.clone()
    }

    #[getter]
    fn storage_config(&self) -> ::core::option::Option<PyStorageConfig> {
        self.0.storage_config.clone().map(PyStorageConfig::from)
    }

    #[getter]
    fn created_at(&self) -> i64 {
        self.0.created_at
    }

    fn __repr__(
        slf: &::pyo3::Bound<'_, Self>,
    ) -> ::pyo3::PyResult<::std::string::String> {
        __python_repr(
            slf.as_any(),
            &[
                ("name", false),
                ("comment", false),
                ("catalog_type", false),
                ("properties", false),
                ("storage_config", false),
                ("created_at", false),
            ],
        )
    }
}

impl ::core::convert::From<super::catalog::v1::Catalog> for PyCatalog {
    fn from(value: super::catalog::v1::Catalog) -> Self {
        Self(value)
    }
}

impl ::core::convert::From<PyCatalog> for super::catalog::v1::Catalog {
    fn from(value: PyCatalog) -> Self {
        value.0
    }
}

#[::pyo3::pyclass(
    name = "CatalogStatus",
    eq,
    frozen,
    module = "example._core.catalog",
    from_py_object
)]
#[derive(Clone, Debug, PartialEq)]
pub struct PyCatalogStatus(pub super::catalog::v1::CatalogStatus);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyCatalogStatus {
    #[getter]
    fn state(&self) -> ::std::string::String {
        self.0.state.clone()
    }

    fn __repr__(
        slf: &::pyo3::Bound<'_, Self>,
    ) -> ::pyo3::PyResult<::std::string::String> {
        __python_repr(slf.as_any(), &[("state", false)])
    }
}

impl ::core::convert::From<super::catalog::v1::CatalogStatus> for PyCatalogStatus {
    fn from(value: super::catalog::v1::CatalogStatus) -> Self {
        Self(value)
    }
}

impl ::core::convert::From<PyCatalogStatus> for super::catalog::v1::CatalogStatus {
    fn from(value: PyCatalogStatus) -> Self {
        value.0
    }
}

#[::pyo3::pyclass(
    name = "CatalogToken",
    eq,
    frozen,
    module = "example._core.catalog",
    from_py_object
)]
#[derive(Clone, Debug, PartialEq)]
pub struct PyCatalogToken(pub super::catalog::v1::CatalogToken);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyCatalogToken {
    #[getter]
    fn token(&self) -> ::std::string::String {
        self.0.token.clone()
    }

    fn __repr__(
        slf: &::pyo3::Bound<'_, Self>,
    ) -> ::pyo3::PyResult<::std::string::String> {
        __python_repr(slf.as_any(), &[("token", false)])
    }
}

impl ::core::convert::From<super::catalog::v1::CatalogToken> for PyCatalogToken {
    fn from(value: super::catalog::v1::CatalogToken) -> Self {
        Self(value)
    }
}

impl ::core::convert::From<PyCatalogToken> for super::catalog::v1::CatalogToken {
    fn from(value: PyCatalogToken) -> Self {
        value.0
    }
}

#[::pyo3::pyclass(
    name = "DeleteCatalogResponse",
    eq,
    frozen,
    module = "example._core.catalog",
    from_py_object
)]
#[derive(Clone, Debug, PartialEq)]
pub struct PyDeleteCatalogResponse(pub super::catalog::v1::DeleteCatalogResponse);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyDeleteCatalogResponse {
    fn __repr__(
        slf: &::pyo3::Bound<'_, Self>,
    ) -> ::pyo3::PyResult<::std::string::String> {
        __python_repr(slf.as_any(), &[])
    }
}

impl ::core::convert::From<super::catalog::v1::DeleteCatalogResponse>
for PyDeleteCatalogResponse {
    fn from(value: super::catalog::v1::DeleteCatalogResponse) -> Self {
        Self(value)
    }
}

impl ::core::convert::From<PyDeleteCatalogResponse>
for super::catalog::v1::DeleteCatalogResponse {
    fn from(value: PyDeleteCatalogResponse) -> Self {
        value.0
    }
}

#[::pyo3::pyclass(
    name = "ListByTagsResponse",
    eq,
    frozen,
    module = "example._core.catalog",
    from_py_object
)]
#[derive(Clone, Debug, PartialEq)]
pub struct PyListByTagsResponse(pub super::catalog::v1::ListByTagsResponse);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyListByTagsResponse {
    #[getter]
    fn results(&self) -> ::std::vec::Vec<::std::string::String> {
        self.0.results.clone()
    }

    fn __repr__(
        slf: &::pyo3::Bound<'_, Self>,
    ) -> ::pyo3::PyResult<::std::string::String> {
        __python_repr(slf.as_any(), &[("results", false)])
    }
}

impl ::core::convert::From<super::catalog::v1::ListByTagsResponse>
for PyListByTagsResponse {
    fn from(value: super::catalog::v1::ListByTagsResponse) -> Self {
        Self(value)
    }
}

impl ::core::convert::From<PyListByTagsResponse>
for super::catalog::v1::ListByTagsResponse {
    fn from(value: PyListByTagsResponse) -> Self {
        value.0
    }
}

#[::pyo3::pyclass(
    name = "ListCatalogsResponse",
    eq,
    frozen,
    module = "example._core.catalog",
    from_py_object
)]
#[derive(Clone, Debug, PartialEq)]
pub struct PyListCatalogsResponse(pub super::catalog::v1::ListCatalogsResponse);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyListCatalogsResponse {
    #[getter]
    fn catalogs(&self) -> ::std::vec::Vec<PyCatalog> {
        self.0.catalogs.iter().cloned().map(PyCatalog::from).collect()
    }

    #[getter]
    fn next_page_token(&self) -> ::std::string::String {
        self.0.next_page_token.clone()
    }

    fn __repr__(
        slf: &::pyo3::Bound<'_, Self>,
    ) -> ::pyo3::PyResult<::std::string::String> {
        __python_repr(slf.as_any(), &[("catalogs", false), ("next_page_token", false)])
    }
}

impl ::core::convert::From<super::catalog::v1::ListCatalogsResponse>
for PyListCatalogsResponse {
    fn from(value: super::catalog::v1::ListCatalogsResponse) -> Self {
        Self(value)
    }
}

impl ::core::convert::From<PyListCatalogsResponse>
for super::catalog::v1::ListCatalogsResponse {
    fn from(value: PyListCatalogsResponse) -> Self {
        value.0
    }
}

#[::pyo3::pyclass(
    name = "S3Config",
    eq,
    frozen,
    module = "example._core.catalog",
    from_py_object
)]
#[derive(Clone, Debug, PartialEq)]
pub struct PyS3Config(pub super::catalog::v1::S3Config);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyS3Config {
    #[new]
    #[pyo3(signature = (bucket = None))]
    fn new(bucket: ::core::option::Option<::std::string::String>) -> Self {
        let mut inner = <super::catalog::v1::S3Config as ::core::default::Default>::default();
        if let ::core::option::Option::Some(value) = bucket {
            inner.bucket = value;
        }
        Self(inner)
    }

    #[getter]
    fn bucket(&self) -> ::std::string::String {
        self.0.bucket.clone()
    }

    fn __repr__(
        slf: &::pyo3::Bound<'_, Self>,
    ) -> ::pyo3::PyResult<::std::string::String> {
        __python_repr(slf.as_any(), &[("bucket", false)])
    }
}

impl ::core::convert::From<super::catalog::v1::S3Config> for PyS3Config {
    fn from(value: super::catalog::v1::S3Config) -> Self {
        Self(value)
    }
}

impl ::core::convert::From<PyS3Config> for super::catalog::v1::S3Config {
    fn from(value: PyS3Config) -> Self {
        value.0
    }
}

#[::pyo3::pyclass(
    name = "StorageConfig",
    eq,
    frozen,
    module = "example._core.catalog",
    from_py_object
)]
#[derive(Clone, Debug, PartialEq)]
pub struct PyStorageConfig(pub super::catalog::v1::StorageConfig);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyStorageConfig {
    #[new]
    #[pyo3(signature = (s3 = None, azure = None))]
    fn new(
        s3: ::core::option::Option<PyS3Config>,
        azure: ::core::option::Option<PyAzureConfig>,
    ) -> Self {
        let mut inner = <super::catalog::v1::StorageConfig as ::core::default::Default>::default();
        if let ::core::option::Option::Some(value) = s3 {
            inner.provider = ::core::option::Option::Some(
                super::catalog::v1::storage_config::Provider::S3(value.into()),
            );
        }
        if let ::core::option::Option::Some(value) = azure {
            inner.provider = ::core::option::Option::Some(
                super::catalog::v1::storage_config::Provider::Azure(value.into()),
            );
        }
        Self(inner)
    }

    #[getter]
    fn s3(&self) -> ::core::option::Option<PyS3Config> {
        match &self.0.provider {
            ::core::option::Option::Some(
                super::catalog::v1::storage_config::Provider::S3(value),
            ) => ::core::option::Option::Some(<PyS3Config>::from((*value).clone())),
            _ => ::core::option::Option::None,
        }
    }

    #[getter]
    fn azure(&self) -> ::core::option::Option<PyAzureConfig> {
        match &self.0.provider {
            ::core::option::Option::Some(
                super::catalog::v1::storage_config::Provider::Azure(value),
            ) => ::core::option::Option::Some(<PyAzureConfig>::from((*value).clone())),
            _ => ::core::option::Option::None,
        }
    }

    fn __repr__(
        slf: &::pyo3::Bound<'_, Self>,
    ) -> ::pyo3::PyResult<::std::string::String> {
        __python_repr(slf.as_any(), &[("s3", false), ("azure", false)])
    }
}

impl ::core::convert::From<super::catalog::v1::StorageConfig> for PyStorageConfig {
    fn from(value: super::catalog::v1::StorageConfig) -> Self {
        Self(value)
    }
}

impl ::core::convert::From<PyStorageConfig> for super::catalog::v1::StorageConfig {
    fn from(value: PyStorageConfig) -> Self {
        value.0
    }
}

#[::pyo3::pyclass(
    name = "DeleteSchemaResponse",
    eq,
    frozen,
    module = "example._core.schemas",
    from_py_object
)]
#[derive(Clone, Debug, PartialEq)]
pub struct PyDeleteSchemaResponse(pub super::schemas::v1::DeleteSchemaResponse);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyDeleteSchemaResponse {
    fn __repr__(
        slf: &::pyo3::Bound<'_, Self>,
    ) -> ::pyo3::PyResult<::std::string::String> {
        __python_repr(slf.as_any(), &[])
    }
}

impl ::core::convert::From<super::schemas::v1::DeleteSchemaResponse>
for PyDeleteSchemaResponse {
    fn from(value: super::schemas::v1::DeleteSchemaResponse) -> Self {
        Self(value)
    }
}

impl ::core::convert::From<PyDeleteSchemaResponse>
for super::schemas::v1::DeleteSchemaResponse {
    fn from(value: PyDeleteSchemaResponse) -> Self {
        value.0
    }
}

#[::pyo3::pyclass(
    name = "ListSchemasResponse",
    eq,
    frozen,
    module = "example._core.schemas",
    from_py_object
)]
#[derive(Clone, Debug, PartialEq)]
pub struct PyListSchemasResponse(pub super::schemas::v1::ListSchemasResponse);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyListSchemasResponse {
    #[getter]
    fn schemas(&self) -> ::std::vec::Vec<PySchema> {
        self.0.schemas.iter().cloned().map(PySchema::from).collect()
    }

    #[getter]
    fn next_page_token(&self) -> ::std::string::String {
        self.0.next_page_token.clone()
    }

    fn __repr__(
        slf: &::pyo3::Bound<'_, Self>,
    ) -> ::pyo3::PyResult<::std::string::String> {
        __python_repr(slf.as_any(), &[("schemas", false), ("next_page_token", false)])
    }
}

impl ::core::convert::From<super::schemas::v1::ListSchemasResponse>
for PyListSchemasResponse {
    fn from(value: super::schemas::v1::ListSchemasResponse) -> Self {
        Self(value)
    }
}

impl ::core::convert::From<PyListSchemasResponse>
for super::schemas::v1::ListSchemasResponse {
    fn from(value: PyListSchemasResponse) -> Self {
        value.0
    }
}

/// A Schema is a child resource of a Catalog. This fixture exercises the generated nested
/// resource-scoped client: `catalog.schema(name)` navigation, `catalog.create_schema(...)`, and the
/// cross-module model import for the create method's enum parameter (`SchemaType`).
#[::pyo3::pyclass(
    name = "Schema",
    eq,
    frozen,
    module = "example._core.schemas",
    from_py_object
)]
#[derive(Clone, Debug, PartialEq)]
pub struct PySchema(pub super::schemas::v1::Schema);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PySchema {
    #[new]
    #[pyo3(
        signature = (
            full_name = None,
            comment = None,
            schema_type = None,
            created_at = None,
            schema_id = None,
            catalog_name = None,
            name = None
        )
    )]
    fn new(
        full_name: ::core::option::Option<::std::string::String>,
        comment: ::core::option::Option<::std::string::String>,
        schema_type: ::core::option::Option<PySchemaType>,
        created_at: ::core::option::Option<i64>,
        schema_id: ::core::option::Option<::std::string::String>,
        catalog_name: ::core::option::Option<::std::string::String>,
        name: ::core::option::Option<::std::string::String>,
    ) -> Self {
        let mut inner = <super::schemas::v1::Schema as ::core::default::Default>::default();
        if let ::core::option::Option::Some(value) = full_name {
            inner.full_name = value;
        }
        if let ::core::option::Option::Some(value) = comment {
            inner.comment = value;
        }
        {
            let value = schema_type;
            inner.schema_type = value
                .map(|e| {
                    <super::schemas::v1::SchemaType as ::core::convert::From<_>>::from(e)
                        as i32
                })
                .unwrap_or(0);
        }
        if let ::core::option::Option::Some(value) = created_at {
            inner.created_at = value;
        }
        if let ::core::option::Option::Some(value) = schema_id {
            inner.schema_id = value;
        }
        if let ::core::option::Option::Some(value) = catalog_name {
            inner.catalog_name = value;
        }
        if let ::core::option::Option::Some(value) = name {
            inner.name = value;
        }
        Self(inner)
    }

    /// Server-computed dot-joined `catalog.schema` composite. OUTPUT_ONLY: the client sends
    /// the component fields (`catalog_name`, `name`), not the join. This field exercises the
    /// generated object-conversion behavior for derived composites — it is stripped before an
    /// object is persisted and recomputed from the components (via `qualified_name()`) on read.
    #[getter]
    fn full_name(&self) -> ::std::string::String {
        self.0.full_name.clone()
    }

    #[getter]
    fn comment(&self) -> ::std::string::String {
        self.0.comment.clone()
    }

    #[getter]
    fn schema_type(&self) -> ::core::option::Option<PySchemaType> {
        <super::schemas::v1::SchemaType as ::core::convert::TryFrom<
            i32,
        >>::try_from(self.0.schema_type)
            .ok()
            .filter(|e| *e as i32 != 0)
            .map(PySchemaType::from)
    }

    #[getter]
    fn created_at(&self) -> i64 {
        self.0.created_at
    }

    /// Stable server-assigned identifier; drives the generated `Object` <-> resource conversions.
    #[getter]
    fn schema_id(&self) -> ::std::string::String {
        self.0.schema_id.clone()
    }

    /// Component name fields the derived `full_name` composite is recomposed from.
    #[getter]
    fn catalog_name(&self) -> ::std::string::String {
        self.0.catalog_name.clone()
    }

    #[getter]
    fn name(&self) -> ::std::string::String {
        self.0.name.clone()
    }

    fn __repr__(
        slf: &::pyo3::Bound<'_, Self>,
    ) -> ::pyo3::PyResult<::std::string::String> {
        __python_repr(
            slf.as_any(),
            &[
                ("full_name", false),
                ("comment", false),
                ("schema_type", false),
                ("created_at", false),
                ("schema_id", false),
                ("catalog_name", false),
                ("name", false),
            ],
        )
    }
}

impl ::core::convert::From<super::schemas::v1::Schema> for PySchema {
    fn from(value: super::schemas::v1::Schema) -> Self {
        Self(value)
    }
}

impl ::core::convert::From<PySchema> for super::schemas::v1::Schema {
    fn from(value: PySchema) -> Self {
        value.0
    }
}

#[::pyo3::pyclass(
    name = "DeleteTagAssignmentResponse",
    eq,
    frozen,
    module = "example._core.tags",
    from_py_object
)]
#[derive(Clone, Debug, PartialEq)]
pub struct PyDeleteTagAssignmentResponse(
    pub super::tags::v1::DeleteTagAssignmentResponse,
);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyDeleteTagAssignmentResponse {
    fn __repr__(
        slf: &::pyo3::Bound<'_, Self>,
    ) -> ::pyo3::PyResult<::std::string::String> {
        __python_repr(slf.as_any(), &[])
    }
}

impl ::core::convert::From<super::tags::v1::DeleteTagAssignmentResponse>
for PyDeleteTagAssignmentResponse {
    fn from(value: super::tags::v1::DeleteTagAssignmentResponse) -> Self {
        Self(value)
    }
}

impl ::core::convert::From<PyDeleteTagAssignmentResponse>
for super::tags::v1::DeleteTagAssignmentResponse {
    fn from(value: PyDeleteTagAssignmentResponse) -> Self {
        value.0
    }
}

#[::pyo3::pyclass(
    name = "ListTagAssignmentsResponse",
    eq,
    frozen,
    module = "example._core.tags",
    from_py_object
)]
#[derive(Clone, Debug, PartialEq)]
pub struct PyListTagAssignmentsResponse(pub super::tags::v1::ListTagAssignmentsResponse);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyListTagAssignmentsResponse {
    #[getter]
    fn tag_assignments(&self) -> ::std::vec::Vec<PyTagAssignment> {
        self.0.tag_assignments.iter().cloned().map(PyTagAssignment::from).collect()
    }

    #[getter]
    fn next_page_token(&self) -> ::std::string::String {
        self.0.next_page_token.clone()
    }

    fn __repr__(
        slf: &::pyo3::Bound<'_, Self>,
    ) -> ::pyo3::PyResult<::std::string::String> {
        __python_repr(
            slf.as_any(),
            &[("tag_assignments", false), ("next_page_token", false)],
        )
    }
}

impl ::core::convert::From<super::tags::v1::ListTagAssignmentsResponse>
for PyListTagAssignmentsResponse {
    fn from(value: super::tags::v1::ListTagAssignmentsResponse) -> Self {
        Self(value)
    }
}

impl ::core::convert::From<PyListTagAssignmentsResponse>
for super::tags::v1::ListTagAssignmentsResponse {
    fn from(value: PyListTagAssignmentsResponse) -> Self {
        value.0
    }
}

#[::pyo3::pyclass(
    name = "TagAssignment",
    eq,
    frozen,
    module = "example._core.tags",
    from_py_object
)]
#[derive(Clone, Debug, PartialEq)]
pub struct PyTagAssignment(pub super::tags::v1::TagAssignment);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyTagAssignment {
    #[new]
    #[pyo3(
        signature = (
            entity_type = None,
            entity_name = None,
            tag_key = None,
            tag_value = None
        )
    )]
    fn new(
        entity_type: ::core::option::Option<::std::string::String>,
        entity_name: ::core::option::Option<::std::string::String>,
        tag_key: ::core::option::Option<::std::string::String>,
        tag_value: ::core::option::Option<::std::string::String>,
    ) -> Self {
        let mut inner = <super::tags::v1::TagAssignment as ::core::default::Default>::default();
        if let ::core::option::Option::Some(value) = entity_type {
            inner.entity_type = value;
        }
        if let ::core::option::Option::Some(value) = entity_name {
            inner.entity_name = value;
        }
        if let ::core::option::Option::Some(value) = tag_key {
            inner.tag_key = value;
        }
        if let ::core::option::Option::Some(value) = tag_value {
            inner.tag_value = value;
        }
        Self(inner)
    }

    #[getter]
    fn entity_type(&self) -> ::std::string::String {
        self.0.entity_type.clone()
    }

    #[getter]
    fn entity_name(&self) -> ::std::string::String {
        self.0.entity_name.clone()
    }

    #[getter]
    fn tag_key(&self) -> ::std::string::String {
        self.0.tag_key.clone()
    }

    #[getter]
    fn tag_value(&self) -> ::std::string::String {
        self.0.tag_value.clone()
    }

    fn __repr__(
        slf: &::pyo3::Bound<'_, Self>,
    ) -> ::pyo3::PyResult<::std::string::String> {
        __python_repr(
            slf.as_any(),
            &[
                ("entity_type", false),
                ("entity_name", false),
                ("tag_key", false),
                ("tag_value", false),
            ],
        )
    }
}

impl ::core::convert::From<super::tags::v1::TagAssignment> for PyTagAssignment {
    fn from(value: super::tags::v1::TagAssignment) -> Self {
        Self(value)
    }
}

impl ::core::convert::From<PyTagAssignment> for super::tags::v1::TagAssignment {
    fn from(value: PyTagAssignment) -> Self {
        value.0
    }
}

/// `Name(field=value, …)` without the `None` fields; a redacted field shows `'***'`.
fn __python_repr(
    obj: &::pyo3::Bound<'_, ::pyo3::PyAny>,
    fields: &[(&str, bool)],
) -> ::pyo3::PyResult<::std::string::String> {
    use ::pyo3::types::{PyAnyMethods, PyTypeMethods};
    let mut shown = ::std::vec::Vec::with_capacity(fields.len());
    for (field, redact) in fields {
        let value = obj.getattr(*field)?;
        if value.is_none() {
            continue;
        }
        if *redact {
            shown.push(::std::format!("{field}='***'"));
        } else {
            shown.push(::std::format!("{field}={}", value.repr() ?));
        }
    }
    Ok(::std::format!("{}({})", obj.get_type().name() ?, shown.join(", ")))
}
