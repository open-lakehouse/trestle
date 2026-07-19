// @generated — do not edit by hand.
#[allow(non_camel_case_types)]
#[::pyo3::pyclass(eq, eq_int, name = "CatalogType", from_py_object)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PyCatalogType {
    CATALOG_TYPE_UNSPECIFIED = 0isize,
    MANAGED_CATALOG = 1isize,
    DELTASHARING_CATALOG = 2isize,
}
impl PyCatalogType {
    fn __to_proto_i32(self) -> i32 {
        match self {
            PyCatalogType::CATALOG_TYPE_UNSPECIFIED => 0i32,
            PyCatalogType::MANAGED_CATALOG => 1i32,
            PyCatalogType::DELTASHARING_CATALOG => 2i32,
        }
    }
    fn __from_proto_i32(value: i32) -> Self {
        match value {
            0i32 => PyCatalogType::CATALOG_TYPE_UNSPECIFIED,
            1i32 => PyCatalogType::MANAGED_CATALOG,
            2i32 => PyCatalogType::DELTASHARING_CATALOG,
            _ => PyCatalogType::CATALOG_TYPE_UNSPECIFIED,
        }
    }
}
impl ::core::convert::From<super::catalog::v1::CatalogType> for PyCatalogType {
    fn from(value: super::catalog::v1::CatalogType) -> Self {
        PyCatalogType::__from_proto_i32(value as i32)
    }
}
impl ::core::convert::From<PyCatalogType> for super::catalog::v1::CatalogType {
    fn from(value: PyCatalogType) -> Self {
        let n = value.__to_proto_i32();
        <super::catalog::v1::CatalogType as ::core::convert::TryFrom<i32>>::try_from(n)
            .unwrap_or_default()
    }
}
#[allow(non_camel_case_types)]
#[::pyo3::pyclass(eq, eq_int, name = "GetSchemaRequestView", from_py_object)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PyGetSchemaRequestView {
    VIEW_UNSPECIFIED = 0isize,
    BASIC = 1isize,
    FULL = 2isize,
}
impl PyGetSchemaRequestView {
    fn __to_proto_i32(self) -> i32 {
        match self {
            PyGetSchemaRequestView::VIEW_UNSPECIFIED => 0i32,
            PyGetSchemaRequestView::BASIC => 1i32,
            PyGetSchemaRequestView::FULL => 2i32,
        }
    }
    fn __from_proto_i32(value: i32) -> Self {
        match value {
            0i32 => PyGetSchemaRequestView::VIEW_UNSPECIFIED,
            1i32 => PyGetSchemaRequestView::BASIC,
            2i32 => PyGetSchemaRequestView::FULL,
            _ => PyGetSchemaRequestView::VIEW_UNSPECIFIED,
        }
    }
}
impl ::core::convert::From<super::schemas::v1::get_schema_request::View>
for PyGetSchemaRequestView {
    fn from(value: super::schemas::v1::get_schema_request::View) -> Self {
        PyGetSchemaRequestView::__from_proto_i32(value as i32)
    }
}
impl ::core::convert::From<PyGetSchemaRequestView>
for super::schemas::v1::get_schema_request::View {
    fn from(value: PyGetSchemaRequestView) -> Self {
        let n = value.__to_proto_i32();
        <super::schemas::v1::get_schema_request::View as ::core::convert::TryFrom<
            i32,
        >>::try_from(n)
            .unwrap_or_default()
    }
}
#[allow(non_camel_case_types)]
#[::pyo3::pyclass(eq, eq_int, name = "SchemaType", from_py_object)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PySchemaType {
    SCHEMA_TYPE_UNSPECIFIED = 0isize,
    MANAGED_SCHEMA = 1isize,
    EXTERNAL_SCHEMA = 2isize,
}
impl PySchemaType {
    fn __to_proto_i32(self) -> i32 {
        match self {
            PySchemaType::SCHEMA_TYPE_UNSPECIFIED => 0i32,
            PySchemaType::MANAGED_SCHEMA => 1i32,
            PySchemaType::EXTERNAL_SCHEMA => 2i32,
        }
    }
    fn __from_proto_i32(value: i32) -> Self {
        match value {
            0i32 => PySchemaType::SCHEMA_TYPE_UNSPECIFIED,
            1i32 => PySchemaType::MANAGED_SCHEMA,
            2i32 => PySchemaType::EXTERNAL_SCHEMA,
            _ => PySchemaType::SCHEMA_TYPE_UNSPECIFIED,
        }
    }
}
impl ::core::convert::From<super::schemas::v1::SchemaType> for PySchemaType {
    fn from(value: super::schemas::v1::SchemaType) -> Self {
        PySchemaType::__from_proto_i32(value as i32)
    }
}
impl ::core::convert::From<PySchemaType> for super::schemas::v1::SchemaType {
    fn from(value: PySchemaType) -> Self {
        let n = value.__to_proto_i32();
        <super::schemas::v1::SchemaType as ::core::convert::TryFrom<i32>>::try_from(n)
            .unwrap_or_default()
    }
}
#[::pyo3::pyclass(name = "AzureConfig", from_py_object)]
#[derive(Clone, Debug)]
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
    #[setter(container)]
    fn set_container(&mut self, value: ::std::string::String) {
        self.0.container = value;
    }
    fn __repr__(&self) -> ::std::string::String {
        ::std::format!("{:?}", self.0)
    }
    fn __eq__(&self, other: &Self) -> bool {
        self.0 == other.0
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
#[::pyo3::pyclass(name = "Catalog", from_py_object)]
#[derive(Clone, Debug)]
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
        if let ::core::option::Option::Some(value) = catalog_type {
            inner.catalog_type = <super::catalog::v1::CatalogType as ::core::convert::From<
                _,
            >>::from(value) as i32;
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
    fn catalog_type(&self) -> PyCatalogType {
        PyCatalogType::from(
            <super::catalog::v1::CatalogType as ::core::convert::TryFrom<
                i32,
            >>::try_from(self.0.catalog_type)
                .unwrap_or_default(),
        )
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
    #[setter(name)]
    fn set_name(&mut self, value: ::std::string::String) {
        self.0.name = value;
    }
    #[setter(comment)]
    fn set_comment(&mut self, value: ::std::string::String) {
        self.0.comment = value;
    }
    #[setter(catalog_type)]
    fn set_catalog_type(&mut self, value: PyCatalogType) {
        self.0.catalog_type = <super::catalog::v1::CatalogType as ::core::convert::From<
            _,
        >>::from(value) as i32;
    }
    #[setter(properties)]
    fn set_properties(
        &mut self,
        value: ::std::collections::HashMap<::std::string::String, ::std::string::String>,
    ) {
        self.0.properties = value;
    }
    #[setter(storage_config)]
    fn set_storage_config(&mut self, value: ::core::option::Option<PyStorageConfig>) {
        self.0.storage_config = value.map(|w| w.into());
    }
    #[setter(created_at)]
    fn set_created_at(&mut self, value: i64) {
        self.0.created_at = value;
    }
    fn __repr__(&self) -> ::std::string::String {
        ::std::format!("{:?}", self.0)
    }
    fn __eq__(&self, other: &Self) -> bool {
        self.0 == other.0
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
#[::pyo3::pyclass(name = "CatalogStatus", from_py_object)]
#[derive(Clone, Debug)]
pub struct PyCatalogStatus(pub super::catalog::v1::CatalogStatus);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyCatalogStatus {
    #[new]
    #[pyo3(signature = (state = None))]
    fn new(state: ::core::option::Option<::std::string::String>) -> Self {
        let mut inner = <super::catalog::v1::CatalogStatus as ::core::default::Default>::default();
        if let ::core::option::Option::Some(value) = state {
            inner.state = value;
        }
        Self(inner)
    }
    #[getter]
    fn state(&self) -> ::std::string::String {
        self.0.state.clone()
    }
    #[setter(state)]
    fn set_state(&mut self, value: ::std::string::String) {
        self.0.state = value;
    }
    fn __repr__(&self) -> ::std::string::String {
        ::std::format!("{:?}", self.0)
    }
    fn __eq__(&self, other: &Self) -> bool {
        self.0 == other.0
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
#[::pyo3::pyclass(name = "CatalogToken", from_py_object)]
#[derive(Clone, Debug)]
pub struct PyCatalogToken(pub super::catalog::v1::CatalogToken);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyCatalogToken {
    #[new]
    #[pyo3(signature = (token = None))]
    fn new(token: ::core::option::Option<::std::string::String>) -> Self {
        let mut inner = <super::catalog::v1::CatalogToken as ::core::default::Default>::default();
        if let ::core::option::Option::Some(value) = token {
            inner.token = value;
        }
        Self(inner)
    }
    #[getter]
    fn token(&self) -> ::std::string::String {
        self.0.token.clone()
    }
    #[setter(token)]
    fn set_token(&mut self, value: ::std::string::String) {
        self.0.token = value;
    }
    fn __repr__(&self) -> ::std::string::String {
        ::std::format!("{:?}", self.0)
    }
    fn __eq__(&self, other: &Self) -> bool {
        self.0 == other.0
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
#[::pyo3::pyclass(name = "CreateCatalogRequest", from_py_object)]
#[derive(Clone, Debug)]
pub struct PyCreateCatalogRequest(pub super::catalog::v1::CreateCatalogRequest);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyCreateCatalogRequest {
    #[new]
    #[pyo3(signature = (catalog = None))]
    fn new(catalog: ::core::option::Option<PyCatalog>) -> Self {
        let mut inner = <super::catalog::v1::CreateCatalogRequest as ::core::default::Default>::default();
        {
            let value = catalog;
            inner.catalog = value.map(|w| w.into());
        }
        Self(inner)
    }
    #[getter]
    fn catalog(&self) -> ::core::option::Option<PyCatalog> {
        self.0.catalog.clone().map(PyCatalog::from)
    }
    #[setter(catalog)]
    fn set_catalog(&mut self, value: ::core::option::Option<PyCatalog>) {
        self.0.catalog = value.map(|w| w.into());
    }
    fn __repr__(&self) -> ::std::string::String {
        ::std::format!("{:?}", self.0)
    }
    fn __eq__(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl ::core::convert::From<super::catalog::v1::CreateCatalogRequest>
for PyCreateCatalogRequest {
    fn from(value: super::catalog::v1::CreateCatalogRequest) -> Self {
        Self(value)
    }
}
impl ::core::convert::From<PyCreateCatalogRequest>
for super::catalog::v1::CreateCatalogRequest {
    fn from(value: PyCreateCatalogRequest) -> Self {
        value.0
    }
}
#[::pyo3::pyclass(name = "DeleteCatalogRequest", from_py_object)]
#[derive(Clone, Debug)]
pub struct PyDeleteCatalogRequest(pub super::catalog::v1::DeleteCatalogRequest);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyDeleteCatalogRequest {
    #[new]
    #[pyo3(signature = (name = None))]
    fn new(name: ::core::option::Option<::std::string::String>) -> Self {
        let mut inner = <super::catalog::v1::DeleteCatalogRequest as ::core::default::Default>::default();
        if let ::core::option::Option::Some(value) = name {
            inner.name = value;
        }
        Self(inner)
    }
    #[getter]
    fn name(&self) -> ::std::string::String {
        self.0.name.clone()
    }
    #[setter(name)]
    fn set_name(&mut self, value: ::std::string::String) {
        self.0.name = value;
    }
    fn __repr__(&self) -> ::std::string::String {
        ::std::format!("{:?}", self.0)
    }
    fn __eq__(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl ::core::convert::From<super::catalog::v1::DeleteCatalogRequest>
for PyDeleteCatalogRequest {
    fn from(value: super::catalog::v1::DeleteCatalogRequest) -> Self {
        Self(value)
    }
}
impl ::core::convert::From<PyDeleteCatalogRequest>
for super::catalog::v1::DeleteCatalogRequest {
    fn from(value: PyDeleteCatalogRequest) -> Self {
        value.0
    }
}
#[::pyo3::pyclass(name = "DeleteCatalogResponse", from_py_object)]
#[derive(Clone, Debug)]
pub struct PyDeleteCatalogResponse(pub super::catalog::v1::DeleteCatalogResponse);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyDeleteCatalogResponse {
    #[new]
    fn new() -> Self {
        Self(::core::default::Default::default())
    }
    fn __repr__(&self) -> ::std::string::String {
        ::std::format!("{:?}", self.0)
    }
    fn __eq__(&self, other: &Self) -> bool {
        self.0 == other.0
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
#[::pyo3::pyclass(name = "GenerateCatalogTokenRequest", from_py_object)]
#[derive(Clone, Debug)]
pub struct PyGenerateCatalogTokenRequest(
    pub super::catalog::v1::GenerateCatalogTokenRequest,
);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyGenerateCatalogTokenRequest {
    #[new]
    #[pyo3(signature = (catalog_id = None))]
    fn new(catalog_id: ::core::option::Option<::std::string::String>) -> Self {
        let mut inner = <super::catalog::v1::GenerateCatalogTokenRequest as ::core::default::Default>::default();
        if let ::core::option::Option::Some(value) = catalog_id {
            inner.catalog_id = value;
        }
        Self(inner)
    }
    #[getter]
    fn catalog_id(&self) -> ::std::string::String {
        self.0.catalog_id.clone()
    }
    #[setter(catalog_id)]
    fn set_catalog_id(&mut self, value: ::std::string::String) {
        self.0.catalog_id = value;
    }
    fn __repr__(&self) -> ::std::string::String {
        ::std::format!("{:?}", self.0)
    }
    fn __eq__(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl ::core::convert::From<super::catalog::v1::GenerateCatalogTokenRequest>
for PyGenerateCatalogTokenRequest {
    fn from(value: super::catalog::v1::GenerateCatalogTokenRequest) -> Self {
        Self(value)
    }
}
impl ::core::convert::From<PyGenerateCatalogTokenRequest>
for super::catalog::v1::GenerateCatalogTokenRequest {
    fn from(value: PyGenerateCatalogTokenRequest) -> Self {
        value.0
    }
}
#[::pyo3::pyclass(name = "GetCatalogRequest", from_py_object)]
#[derive(Clone, Debug)]
pub struct PyGetCatalogRequest(pub super::catalog::v1::GetCatalogRequest);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyGetCatalogRequest {
    #[new]
    #[pyo3(signature = (name = None))]
    fn new(name: ::core::option::Option<::std::string::String>) -> Self {
        let mut inner = <super::catalog::v1::GetCatalogRequest as ::core::default::Default>::default();
        if let ::core::option::Option::Some(value) = name {
            inner.name = value;
        }
        Self(inner)
    }
    #[getter]
    fn name(&self) -> ::std::string::String {
        self.0.name.clone()
    }
    #[setter(name)]
    fn set_name(&mut self, value: ::std::string::String) {
        self.0.name = value;
    }
    fn __repr__(&self) -> ::std::string::String {
        ::std::format!("{:?}", self.0)
    }
    fn __eq__(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl ::core::convert::From<super::catalog::v1::GetCatalogRequest>
for PyGetCatalogRequest {
    fn from(value: super::catalog::v1::GetCatalogRequest) -> Self {
        Self(value)
    }
}
impl ::core::convert::From<PyGetCatalogRequest>
for super::catalog::v1::GetCatalogRequest {
    fn from(value: PyGetCatalogRequest) -> Self {
        value.0
    }
}
#[::pyo3::pyclass(name = "GetCatalogStatusRequest", from_py_object)]
#[derive(Clone, Debug)]
pub struct PyGetCatalogStatusRequest(pub super::catalog::v1::GetCatalogStatusRequest);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyGetCatalogStatusRequest {
    #[new]
    #[pyo3(signature = (name = None))]
    fn new(name: ::core::option::Option<::std::string::String>) -> Self {
        let mut inner = <super::catalog::v1::GetCatalogStatusRequest as ::core::default::Default>::default();
        if let ::core::option::Option::Some(value) = name {
            inner.name = value;
        }
        Self(inner)
    }
    #[getter]
    fn name(&self) -> ::std::string::String {
        self.0.name.clone()
    }
    #[setter(name)]
    fn set_name(&mut self, value: ::std::string::String) {
        self.0.name = value;
    }
    fn __repr__(&self) -> ::std::string::String {
        ::std::format!("{:?}", self.0)
    }
    fn __eq__(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl ::core::convert::From<super::catalog::v1::GetCatalogStatusRequest>
for PyGetCatalogStatusRequest {
    fn from(value: super::catalog::v1::GetCatalogStatusRequest) -> Self {
        Self(value)
    }
}
impl ::core::convert::From<PyGetCatalogStatusRequest>
for super::catalog::v1::GetCatalogStatusRequest {
    fn from(value: PyGetCatalogStatusRequest) -> Self {
        value.0
    }
}
#[::pyo3::pyclass(name = "ListByCatalogTypeRequest", from_py_object)]
#[derive(Clone, Debug)]
pub struct PyListByCatalogTypeRequest(pub super::catalog::v1::ListByCatalogTypeRequest);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyListByCatalogTypeRequest {
    #[new]
    #[pyo3(signature = (catalog_type = None))]
    fn new(catalog_type: ::core::option::Option<PyCatalogType>) -> Self {
        let mut inner = <super::catalog::v1::ListByCatalogTypeRequest as ::core::default::Default>::default();
        if let ::core::option::Option::Some(value) = catalog_type {
            inner.catalog_type = <super::catalog::v1::CatalogType as ::core::convert::From<
                _,
            >>::from(value) as i32;
        }
        Self(inner)
    }
    #[getter]
    fn catalog_type(&self) -> PyCatalogType {
        PyCatalogType::from(
            <super::catalog::v1::CatalogType as ::core::convert::TryFrom<
                i32,
            >>::try_from(self.0.catalog_type)
                .unwrap_or_default(),
        )
    }
    #[setter(catalog_type)]
    fn set_catalog_type(&mut self, value: PyCatalogType) {
        self.0.catalog_type = <super::catalog::v1::CatalogType as ::core::convert::From<
            _,
        >>::from(value) as i32;
    }
    fn __repr__(&self) -> ::std::string::String {
        ::std::format!("{:?}", self.0)
    }
    fn __eq__(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl ::core::convert::From<super::catalog::v1::ListByCatalogTypeRequest>
for PyListByCatalogTypeRequest {
    fn from(value: super::catalog::v1::ListByCatalogTypeRequest) -> Self {
        Self(value)
    }
}
impl ::core::convert::From<PyListByCatalogTypeRequest>
for super::catalog::v1::ListByCatalogTypeRequest {
    fn from(value: PyListByCatalogTypeRequest) -> Self {
        value.0
    }
}
#[::pyo3::pyclass(name = "ListByTagsRequest", from_py_object)]
#[derive(Clone, Debug)]
pub struct PyListByTagsRequest(pub super::catalog::v1::ListByTagsRequest);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyListByTagsRequest {
    #[new]
    #[pyo3(signature = (tags = None, max_results = None))]
    fn new(
        tags: ::core::option::Option<::std::vec::Vec<::std::string::String>>,
        max_results: ::core::option::Option<i32>,
    ) -> Self {
        let mut inner = <super::catalog::v1::ListByTagsRequest as ::core::default::Default>::default();
        if let ::core::option::Option::Some(value) = tags {
            inner.tags = value;
        }
        if let ::core::option::Option::Some(value) = max_results {
            inner.max_results = value;
        }
        Self(inner)
    }
    #[getter]
    fn tags(&self) -> ::std::vec::Vec<::std::string::String> {
        self.0.tags.clone()
    }
    #[getter]
    fn max_results(&self) -> i32 {
        self.0.max_results
    }
    #[setter(tags)]
    fn set_tags(&mut self, value: ::std::vec::Vec<::std::string::String>) {
        self.0.tags = value;
    }
    #[setter(max_results)]
    fn set_max_results(&mut self, value: i32) {
        self.0.max_results = value;
    }
    fn __repr__(&self) -> ::std::string::String {
        ::std::format!("{:?}", self.0)
    }
    fn __eq__(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl ::core::convert::From<super::catalog::v1::ListByTagsRequest>
for PyListByTagsRequest {
    fn from(value: super::catalog::v1::ListByTagsRequest) -> Self {
        Self(value)
    }
}
impl ::core::convert::From<PyListByTagsRequest>
for super::catalog::v1::ListByTagsRequest {
    fn from(value: PyListByTagsRequest) -> Self {
        value.0
    }
}
#[::pyo3::pyclass(name = "ListByTagsResponse", from_py_object)]
#[derive(Clone, Debug)]
pub struct PyListByTagsResponse(pub super::catalog::v1::ListByTagsResponse);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyListByTagsResponse {
    #[new]
    #[pyo3(signature = (results = None))]
    fn new(
        results: ::core::option::Option<::std::vec::Vec<::std::string::String>>,
    ) -> Self {
        let mut inner = <super::catalog::v1::ListByTagsResponse as ::core::default::Default>::default();
        if let ::core::option::Option::Some(value) = results {
            inner.results = value;
        }
        Self(inner)
    }
    #[getter]
    fn results(&self) -> ::std::vec::Vec<::std::string::String> {
        self.0.results.clone()
    }
    #[setter(results)]
    fn set_results(&mut self, value: ::std::vec::Vec<::std::string::String>) {
        self.0.results = value;
    }
    fn __repr__(&self) -> ::std::string::String {
        ::std::format!("{:?}", self.0)
    }
    fn __eq__(&self, other: &Self) -> bool {
        self.0 == other.0
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
#[::pyo3::pyclass(name = "ListCatalogsRequest", from_py_object)]
#[derive(Clone, Debug)]
pub struct PyListCatalogsRequest(pub super::catalog::v1::ListCatalogsRequest);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyListCatalogsRequest {
    #[new]
    #[pyo3(signature = (max_results = None, page_token = None))]
    fn new(
        max_results: ::core::option::Option<i32>,
        page_token: ::core::option::Option<::std::string::String>,
    ) -> Self {
        let mut inner = <super::catalog::v1::ListCatalogsRequest as ::core::default::Default>::default();
        if let ::core::option::Option::Some(value) = max_results {
            inner.max_results = value;
        }
        if let ::core::option::Option::Some(value) = page_token {
            inner.page_token = value;
        }
        Self(inner)
    }
    #[getter]
    fn max_results(&self) -> i32 {
        self.0.max_results
    }
    #[getter]
    fn page_token(&self) -> ::std::string::String {
        self.0.page_token.clone()
    }
    #[setter(max_results)]
    fn set_max_results(&mut self, value: i32) {
        self.0.max_results = value;
    }
    #[setter(page_token)]
    fn set_page_token(&mut self, value: ::std::string::String) {
        self.0.page_token = value;
    }
    fn __repr__(&self) -> ::std::string::String {
        ::std::format!("{:?}", self.0)
    }
    fn __eq__(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl ::core::convert::From<super::catalog::v1::ListCatalogsRequest>
for PyListCatalogsRequest {
    fn from(value: super::catalog::v1::ListCatalogsRequest) -> Self {
        Self(value)
    }
}
impl ::core::convert::From<PyListCatalogsRequest>
for super::catalog::v1::ListCatalogsRequest {
    fn from(value: PyListCatalogsRequest) -> Self {
        value.0
    }
}
#[::pyo3::pyclass(name = "ListCatalogsResponse", from_py_object)]
#[derive(Clone, Debug)]
pub struct PyListCatalogsResponse(pub super::catalog::v1::ListCatalogsResponse);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyListCatalogsResponse {
    #[new]
    #[pyo3(signature = (catalogs = None, next_page_token = None))]
    fn new(
        catalogs: ::core::option::Option<::std::vec::Vec<PyCatalog>>,
        next_page_token: ::core::option::Option<::std::string::String>,
    ) -> Self {
        let mut inner = <super::catalog::v1::ListCatalogsResponse as ::core::default::Default>::default();
        if let ::core::option::Option::Some(value) = catalogs {
            inner.catalogs = value
                .into_iter()
                .map(::core::convert::Into::into)
                .collect();
        }
        if let ::core::option::Option::Some(value) = next_page_token {
            inner.next_page_token = value;
        }
        Self(inner)
    }
    #[getter]
    fn catalogs(&self) -> ::std::vec::Vec<PyCatalog> {
        self.0.catalogs.iter().cloned().map(PyCatalog::from).collect()
    }
    #[getter]
    fn next_page_token(&self) -> ::std::string::String {
        self.0.next_page_token.clone()
    }
    #[setter(catalogs)]
    fn set_catalogs(&mut self, value: ::std::vec::Vec<PyCatalog>) {
        self.0.catalogs = value.into_iter().map(::core::convert::Into::into).collect();
    }
    #[setter(next_page_token)]
    fn set_next_page_token(&mut self, value: ::std::string::String) {
        self.0.next_page_token = value;
    }
    fn __repr__(&self) -> ::std::string::String {
        ::std::format!("{:?}", self.0)
    }
    fn __eq__(&self, other: &Self) -> bool {
        self.0 == other.0
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
#[::pyo3::pyclass(name = "S3Config", from_py_object)]
#[derive(Clone, Debug)]
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
    #[setter(bucket)]
    fn set_bucket(&mut self, value: ::std::string::String) {
        self.0.bucket = value;
    }
    fn __repr__(&self) -> ::std::string::String {
        ::std::format!("{:?}", self.0)
    }
    fn __eq__(&self, other: &Self) -> bool {
        self.0 == other.0
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
#[::pyo3::pyclass(name = "StorageConfig", from_py_object)]
#[derive(Clone, Debug)]
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
    #[setter(s3)]
    fn set_s3(&mut self, value: ::core::option::Option<PyS3Config>) {
        match value {
            ::core::option::Option::Some(value) => {
                self.0.provider = ::core::option::Option::Some(
                    super::catalog::v1::storage_config::Provider::S3(value.into()),
                );
            }
            ::core::option::Option::None => {
                if ::core::matches!(
                    self.0. provider,
                    ::core::option::Option::Some(super::catalog::v1::storage_config::Provider::S3(_))
                ) {
                    self.0.provider = ::core::option::Option::None;
                }
            }
        }
    }
    #[setter(azure)]
    fn set_azure(&mut self, value: ::core::option::Option<PyAzureConfig>) {
        match value {
            ::core::option::Option::Some(value) => {
                self.0.provider = ::core::option::Option::Some(
                    super::catalog::v1::storage_config::Provider::Azure(value.into()),
                );
            }
            ::core::option::Option::None => {
                if ::core::matches!(
                    self.0. provider,
                    ::core::option::Option::Some(super::catalog::v1::storage_config::Provider::Azure(_))
                ) {
                    self.0.provider = ::core::option::Option::None;
                }
            }
        }
    }
    fn __repr__(&self) -> ::std::string::String {
        ::std::format!("{:?}", self.0)
    }
    fn __eq__(&self, other: &Self) -> bool {
        self.0 == other.0
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
#[::pyo3::pyclass(name = "UpdateCatalogRequest", from_py_object)]
#[derive(Clone, Debug)]
pub struct PyUpdateCatalogRequest(pub super::catalog::v1::UpdateCatalogRequest);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyUpdateCatalogRequest {
    #[new]
    #[pyo3(signature = (name = None, catalog = None))]
    fn new(
        name: ::core::option::Option<::std::string::String>,
        catalog: ::core::option::Option<PyCatalog>,
    ) -> Self {
        let mut inner = <super::catalog::v1::UpdateCatalogRequest as ::core::default::Default>::default();
        if let ::core::option::Option::Some(value) = name {
            inner.name = value;
        }
        {
            let value = catalog;
            inner.catalog = value.map(|w| w.into());
        }
        Self(inner)
    }
    #[getter]
    fn name(&self) -> ::std::string::String {
        self.0.name.clone()
    }
    #[getter]
    fn catalog(&self) -> ::core::option::Option<PyCatalog> {
        self.0.catalog.clone().map(PyCatalog::from)
    }
    #[setter(name)]
    fn set_name(&mut self, value: ::std::string::String) {
        self.0.name = value;
    }
    #[setter(catalog)]
    fn set_catalog(&mut self, value: ::core::option::Option<PyCatalog>) {
        self.0.catalog = value.map(|w| w.into());
    }
    fn __repr__(&self) -> ::std::string::String {
        ::std::format!("{:?}", self.0)
    }
    fn __eq__(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl ::core::convert::From<super::catalog::v1::UpdateCatalogRequest>
for PyUpdateCatalogRequest {
    fn from(value: super::catalog::v1::UpdateCatalogRequest) -> Self {
        Self(value)
    }
}
impl ::core::convert::From<PyUpdateCatalogRequest>
for super::catalog::v1::UpdateCatalogRequest {
    fn from(value: PyUpdateCatalogRequest) -> Self {
        value.0
    }
}
#[::pyo3::pyclass(name = "CreateSchemaRequest", from_py_object)]
#[derive(Clone, Debug)]
pub struct PyCreateSchemaRequest(pub super::schemas::v1::CreateSchemaRequest);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyCreateSchemaRequest {
    #[new]
    #[pyo3(signature = (name = None, catalog_name = None, schema_type = None))]
    fn new(
        name: ::core::option::Option<::std::string::String>,
        catalog_name: ::core::option::Option<::std::string::String>,
        schema_type: ::core::option::Option<PySchemaType>,
    ) -> Self {
        let mut inner = <super::schemas::v1::CreateSchemaRequest as ::core::default::Default>::default();
        if let ::core::option::Option::Some(value) = name {
            inner.name = value;
        }
        if let ::core::option::Option::Some(value) = catalog_name {
            inner.catalog_name = value;
        }
        if let ::core::option::Option::Some(value) = schema_type {
            inner.schema_type = <super::schemas::v1::SchemaType as ::core::convert::From<
                _,
            >>::from(value) as i32;
        }
        Self(inner)
    }
    #[getter]
    fn name(&self) -> ::std::string::String {
        self.0.name.clone()
    }
    #[getter]
    fn catalog_name(&self) -> ::std::string::String {
        self.0.catalog_name.clone()
    }
    #[getter]
    fn schema_type(&self) -> PySchemaType {
        PySchemaType::from(
            <super::schemas::v1::SchemaType as ::core::convert::TryFrom<
                i32,
            >>::try_from(self.0.schema_type)
                .unwrap_or_default(),
        )
    }
    #[setter(name)]
    fn set_name(&mut self, value: ::std::string::String) {
        self.0.name = value;
    }
    #[setter(catalog_name)]
    fn set_catalog_name(&mut self, value: ::std::string::String) {
        self.0.catalog_name = value;
    }
    #[setter(schema_type)]
    fn set_schema_type(&mut self, value: PySchemaType) {
        self.0.schema_type = <super::schemas::v1::SchemaType as ::core::convert::From<
            _,
        >>::from(value) as i32;
    }
    fn __repr__(&self) -> ::std::string::String {
        ::std::format!("{:?}", self.0)
    }
    fn __eq__(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl ::core::convert::From<super::schemas::v1::CreateSchemaRequest>
for PyCreateSchemaRequest {
    fn from(value: super::schemas::v1::CreateSchemaRequest) -> Self {
        Self(value)
    }
}
impl ::core::convert::From<PyCreateSchemaRequest>
for super::schemas::v1::CreateSchemaRequest {
    fn from(value: PyCreateSchemaRequest) -> Self {
        value.0
    }
}
#[::pyo3::pyclass(name = "DeleteSchemaRequest", from_py_object)]
#[derive(Clone, Debug)]
pub struct PyDeleteSchemaRequest(pub super::schemas::v1::DeleteSchemaRequest);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyDeleteSchemaRequest {
    #[new]
    #[pyo3(signature = (full_name = None))]
    fn new(full_name: ::core::option::Option<::std::string::String>) -> Self {
        let mut inner = <super::schemas::v1::DeleteSchemaRequest as ::core::default::Default>::default();
        if let ::core::option::Option::Some(value) = full_name {
            inner.full_name = value;
        }
        Self(inner)
    }
    #[getter]
    fn full_name(&self) -> ::std::string::String {
        self.0.full_name.clone()
    }
    #[setter(full_name)]
    fn set_full_name(&mut self, value: ::std::string::String) {
        self.0.full_name = value;
    }
    fn __repr__(&self) -> ::std::string::String {
        ::std::format!("{:?}", self.0)
    }
    fn __eq__(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl ::core::convert::From<super::schemas::v1::DeleteSchemaRequest>
for PyDeleteSchemaRequest {
    fn from(value: super::schemas::v1::DeleteSchemaRequest) -> Self {
        Self(value)
    }
}
impl ::core::convert::From<PyDeleteSchemaRequest>
for super::schemas::v1::DeleteSchemaRequest {
    fn from(value: PyDeleteSchemaRequest) -> Self {
        value.0
    }
}
#[::pyo3::pyclass(name = "DeleteSchemaResponse", from_py_object)]
#[derive(Clone, Debug)]
pub struct PyDeleteSchemaResponse(pub super::schemas::v1::DeleteSchemaResponse);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyDeleteSchemaResponse {
    #[new]
    fn new() -> Self {
        Self(::core::default::Default::default())
    }
    fn __repr__(&self) -> ::std::string::String {
        ::std::format!("{:?}", self.0)
    }
    fn __eq__(&self, other: &Self) -> bool {
        self.0 == other.0
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
#[::pyo3::pyclass(name = "GetSchemaRequest", from_py_object)]
#[derive(Clone, Debug)]
pub struct PyGetSchemaRequest(pub super::schemas::v1::GetSchemaRequest);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyGetSchemaRequest {
    #[new]
    #[pyo3(signature = (full_name = None, view = None))]
    fn new(
        full_name: ::core::option::Option<::std::string::String>,
        view: ::core::option::Option<PyGetSchemaRequestView>,
    ) -> Self {
        let mut inner = <super::schemas::v1::GetSchemaRequest as ::core::default::Default>::default();
        if let ::core::option::Option::Some(value) = full_name {
            inner.full_name = value;
        }
        if let ::core::option::Option::Some(value) = view {
            inner.view = <super::schemas::v1::get_schema_request::View as ::core::convert::From<
                _,
            >>::from(value) as i32;
        }
        Self(inner)
    }
    #[getter]
    fn full_name(&self) -> ::std::string::String {
        self.0.full_name.clone()
    }
    #[getter]
    fn view(&self) -> PyGetSchemaRequestView {
        PyGetSchemaRequestView::from(
            <super::schemas::v1::get_schema_request::View as ::core::convert::TryFrom<
                i32,
            >>::try_from(self.0.view)
                .unwrap_or_default(),
        )
    }
    #[setter(full_name)]
    fn set_full_name(&mut self, value: ::std::string::String) {
        self.0.full_name = value;
    }
    #[setter(view)]
    fn set_view(&mut self, value: PyGetSchemaRequestView) {
        self.0.view = <super::schemas::v1::get_schema_request::View as ::core::convert::From<
            _,
        >>::from(value) as i32;
    }
    fn __repr__(&self) -> ::std::string::String {
        ::std::format!("{:?}", self.0)
    }
    fn __eq__(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl ::core::convert::From<super::schemas::v1::GetSchemaRequest> for PyGetSchemaRequest {
    fn from(value: super::schemas::v1::GetSchemaRequest) -> Self {
        Self(value)
    }
}
impl ::core::convert::From<PyGetSchemaRequest> for super::schemas::v1::GetSchemaRequest {
    fn from(value: PyGetSchemaRequest) -> Self {
        value.0
    }
}
#[::pyo3::pyclass(name = "ListSchemasRequest", from_py_object)]
#[derive(Clone, Debug)]
pub struct PyListSchemasRequest(pub super::schemas::v1::ListSchemasRequest);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyListSchemasRequest {
    #[new]
    #[pyo3(signature = (catalog_name = None, max_results = None, page_token = None))]
    fn new(
        catalog_name: ::core::option::Option<::std::string::String>,
        max_results: ::core::option::Option<i32>,
        page_token: ::core::option::Option<::std::string::String>,
    ) -> Self {
        let mut inner = <super::schemas::v1::ListSchemasRequest as ::core::default::Default>::default();
        if let ::core::option::Option::Some(value) = catalog_name {
            inner.catalog_name = value;
        }
        if let ::core::option::Option::Some(value) = max_results {
            inner.max_results = value;
        }
        if let ::core::option::Option::Some(value) = page_token {
            inner.page_token = value;
        }
        Self(inner)
    }
    #[getter]
    fn catalog_name(&self) -> ::std::string::String {
        self.0.catalog_name.clone()
    }
    #[getter]
    fn max_results(&self) -> i32 {
        self.0.max_results
    }
    #[getter]
    fn page_token(&self) -> ::std::string::String {
        self.0.page_token.clone()
    }
    #[setter(catalog_name)]
    fn set_catalog_name(&mut self, value: ::std::string::String) {
        self.0.catalog_name = value;
    }
    #[setter(max_results)]
    fn set_max_results(&mut self, value: i32) {
        self.0.max_results = value;
    }
    #[setter(page_token)]
    fn set_page_token(&mut self, value: ::std::string::String) {
        self.0.page_token = value;
    }
    fn __repr__(&self) -> ::std::string::String {
        ::std::format!("{:?}", self.0)
    }
    fn __eq__(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl ::core::convert::From<super::schemas::v1::ListSchemasRequest>
for PyListSchemasRequest {
    fn from(value: super::schemas::v1::ListSchemasRequest) -> Self {
        Self(value)
    }
}
impl ::core::convert::From<PyListSchemasRequest>
for super::schemas::v1::ListSchemasRequest {
    fn from(value: PyListSchemasRequest) -> Self {
        value.0
    }
}
#[::pyo3::pyclass(name = "ListSchemasResponse", from_py_object)]
#[derive(Clone, Debug)]
pub struct PyListSchemasResponse(pub super::schemas::v1::ListSchemasResponse);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyListSchemasResponse {
    #[new]
    #[pyo3(signature = (schemas = None, next_page_token = None))]
    fn new(
        schemas: ::core::option::Option<::std::vec::Vec<PySchema>>,
        next_page_token: ::core::option::Option<::std::string::String>,
    ) -> Self {
        let mut inner = <super::schemas::v1::ListSchemasResponse as ::core::default::Default>::default();
        if let ::core::option::Option::Some(value) = schemas {
            inner.schemas = value.into_iter().map(::core::convert::Into::into).collect();
        }
        if let ::core::option::Option::Some(value) = next_page_token {
            inner.next_page_token = value;
        }
        Self(inner)
    }
    #[getter]
    fn schemas(&self) -> ::std::vec::Vec<PySchema> {
        self.0.schemas.iter().cloned().map(PySchema::from).collect()
    }
    #[getter]
    fn next_page_token(&self) -> ::std::string::String {
        self.0.next_page_token.clone()
    }
    #[setter(schemas)]
    fn set_schemas(&mut self, value: ::std::vec::Vec<PySchema>) {
        self.0.schemas = value.into_iter().map(::core::convert::Into::into).collect();
    }
    #[setter(next_page_token)]
    fn set_next_page_token(&mut self, value: ::std::string::String) {
        self.0.next_page_token = value;
    }
    fn __repr__(&self) -> ::std::string::String {
        ::std::format!("{:?}", self.0)
    }
    fn __eq__(&self, other: &Self) -> bool {
        self.0 == other.0
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
#[::pyo3::pyclass(name = "Schema", from_py_object)]
#[derive(Clone, Debug)]
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
        if let ::core::option::Option::Some(value) = schema_type {
            inner.schema_type = <super::schemas::v1::SchemaType as ::core::convert::From<
                _,
            >>::from(value) as i32;
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
    #[getter]
    fn full_name(&self) -> ::std::string::String {
        self.0.full_name.clone()
    }
    #[getter]
    fn comment(&self) -> ::std::string::String {
        self.0.comment.clone()
    }
    #[getter]
    fn schema_type(&self) -> PySchemaType {
        PySchemaType::from(
            <super::schemas::v1::SchemaType as ::core::convert::TryFrom<
                i32,
            >>::try_from(self.0.schema_type)
                .unwrap_or_default(),
        )
    }
    #[getter]
    fn created_at(&self) -> i64 {
        self.0.created_at
    }
    #[getter]
    fn schema_id(&self) -> ::std::string::String {
        self.0.schema_id.clone()
    }
    #[getter]
    fn catalog_name(&self) -> ::std::string::String {
        self.0.catalog_name.clone()
    }
    #[getter]
    fn name(&self) -> ::std::string::String {
        self.0.name.clone()
    }
    #[setter(full_name)]
    fn set_full_name(&mut self, value: ::std::string::String) {
        self.0.full_name = value;
    }
    #[setter(comment)]
    fn set_comment(&mut self, value: ::std::string::String) {
        self.0.comment = value;
    }
    #[setter(schema_type)]
    fn set_schema_type(&mut self, value: PySchemaType) {
        self.0.schema_type = <super::schemas::v1::SchemaType as ::core::convert::From<
            _,
        >>::from(value) as i32;
    }
    #[setter(created_at)]
    fn set_created_at(&mut self, value: i64) {
        self.0.created_at = value;
    }
    #[setter(schema_id)]
    fn set_schema_id(&mut self, value: ::std::string::String) {
        self.0.schema_id = value;
    }
    #[setter(catalog_name)]
    fn set_catalog_name(&mut self, value: ::std::string::String) {
        self.0.catalog_name = value;
    }
    #[setter(name)]
    fn set_name(&mut self, value: ::std::string::String) {
        self.0.name = value;
    }
    fn __repr__(&self) -> ::std::string::String {
        ::std::format!("{:?}", self.0)
    }
    fn __eq__(&self, other: &Self) -> bool {
        self.0 == other.0
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
#[::pyo3::pyclass(name = "UpdateSchemaRequest", from_py_object)]
#[derive(Clone, Debug)]
pub struct PyUpdateSchemaRequest(pub super::schemas::v1::UpdateSchemaRequest);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyUpdateSchemaRequest {
    #[new]
    #[pyo3(signature = (full_name = None, schema = None))]
    fn new(
        full_name: ::core::option::Option<::std::string::String>,
        schema: ::core::option::Option<PySchema>,
    ) -> Self {
        let mut inner = <super::schemas::v1::UpdateSchemaRequest as ::core::default::Default>::default();
        if let ::core::option::Option::Some(value) = full_name {
            inner.full_name = value;
        }
        {
            let value = schema;
            inner.schema = value.map(|w| w.into());
        }
        Self(inner)
    }
    #[getter]
    fn full_name(&self) -> ::std::string::String {
        self.0.full_name.clone()
    }
    #[getter]
    fn schema(&self) -> ::core::option::Option<PySchema> {
        self.0.schema.clone().map(PySchema::from)
    }
    #[setter(full_name)]
    fn set_full_name(&mut self, value: ::std::string::String) {
        self.0.full_name = value;
    }
    #[setter(schema)]
    fn set_schema(&mut self, value: ::core::option::Option<PySchema>) {
        self.0.schema = value.map(|w| w.into());
    }
    fn __repr__(&self) -> ::std::string::String {
        ::std::format!("{:?}", self.0)
    }
    fn __eq__(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl ::core::convert::From<super::schemas::v1::UpdateSchemaRequest>
for PyUpdateSchemaRequest {
    fn from(value: super::schemas::v1::UpdateSchemaRequest) -> Self {
        Self(value)
    }
}
impl ::core::convert::From<PyUpdateSchemaRequest>
for super::schemas::v1::UpdateSchemaRequest {
    fn from(value: PyUpdateSchemaRequest) -> Self {
        value.0
    }
}
#[::pyo3::pyclass(name = "CreateTagAssignmentRequest", from_py_object)]
#[derive(Clone, Debug)]
pub struct PyCreateTagAssignmentRequest(pub super::tags::v1::CreateTagAssignmentRequest);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyCreateTagAssignmentRequest {
    #[new]
    #[pyo3(signature = (entity_type = None, entity_name = None, tag = None))]
    fn new(
        entity_type: ::core::option::Option<::std::string::String>,
        entity_name: ::core::option::Option<::std::string::String>,
        tag: ::core::option::Option<PyTagAssignment>,
    ) -> Self {
        let mut inner = <super::tags::v1::CreateTagAssignmentRequest as ::core::default::Default>::default();
        if let ::core::option::Option::Some(value) = entity_type {
            inner.entity_type = value;
        }
        if let ::core::option::Option::Some(value) = entity_name {
            inner.entity_name = value;
        }
        {
            let value = tag;
            inner.tag = value.map(|w| w.into());
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
    fn tag(&self) -> ::core::option::Option<PyTagAssignment> {
        self.0.tag.clone().map(PyTagAssignment::from)
    }
    #[setter(entity_type)]
    fn set_entity_type(&mut self, value: ::std::string::String) {
        self.0.entity_type = value;
    }
    #[setter(entity_name)]
    fn set_entity_name(&mut self, value: ::std::string::String) {
        self.0.entity_name = value;
    }
    #[setter(tag)]
    fn set_tag(&mut self, value: ::core::option::Option<PyTagAssignment>) {
        self.0.tag = value.map(|w| w.into());
    }
    fn __repr__(&self) -> ::std::string::String {
        ::std::format!("{:?}", self.0)
    }
    fn __eq__(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl ::core::convert::From<super::tags::v1::CreateTagAssignmentRequest>
for PyCreateTagAssignmentRequest {
    fn from(value: super::tags::v1::CreateTagAssignmentRequest) -> Self {
        Self(value)
    }
}
impl ::core::convert::From<PyCreateTagAssignmentRequest>
for super::tags::v1::CreateTagAssignmentRequest {
    fn from(value: PyCreateTagAssignmentRequest) -> Self {
        value.0
    }
}
#[::pyo3::pyclass(name = "DeleteTagAssignmentRequest", from_py_object)]
#[derive(Clone, Debug)]
pub struct PyDeleteTagAssignmentRequest(pub super::tags::v1::DeleteTagAssignmentRequest);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyDeleteTagAssignmentRequest {
    #[new]
    #[pyo3(signature = (entity_type = None, entity_name = None, tag_key = None))]
    fn new(
        entity_type: ::core::option::Option<::std::string::String>,
        entity_name: ::core::option::Option<::std::string::String>,
        tag_key: ::core::option::Option<::std::string::String>,
    ) -> Self {
        let mut inner = <super::tags::v1::DeleteTagAssignmentRequest as ::core::default::Default>::default();
        if let ::core::option::Option::Some(value) = entity_type {
            inner.entity_type = value;
        }
        if let ::core::option::Option::Some(value) = entity_name {
            inner.entity_name = value;
        }
        if let ::core::option::Option::Some(value) = tag_key {
            inner.tag_key = value;
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
    #[setter(entity_type)]
    fn set_entity_type(&mut self, value: ::std::string::String) {
        self.0.entity_type = value;
    }
    #[setter(entity_name)]
    fn set_entity_name(&mut self, value: ::std::string::String) {
        self.0.entity_name = value;
    }
    #[setter(tag_key)]
    fn set_tag_key(&mut self, value: ::std::string::String) {
        self.0.tag_key = value;
    }
    fn __repr__(&self) -> ::std::string::String {
        ::std::format!("{:?}", self.0)
    }
    fn __eq__(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl ::core::convert::From<super::tags::v1::DeleteTagAssignmentRequest>
for PyDeleteTagAssignmentRequest {
    fn from(value: super::tags::v1::DeleteTagAssignmentRequest) -> Self {
        Self(value)
    }
}
impl ::core::convert::From<PyDeleteTagAssignmentRequest>
for super::tags::v1::DeleteTagAssignmentRequest {
    fn from(value: PyDeleteTagAssignmentRequest) -> Self {
        value.0
    }
}
#[::pyo3::pyclass(name = "DeleteTagAssignmentResponse", from_py_object)]
#[derive(Clone, Debug)]
pub struct PyDeleteTagAssignmentResponse(
    pub super::tags::v1::DeleteTagAssignmentResponse,
);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyDeleteTagAssignmentResponse {
    #[new]
    fn new() -> Self {
        Self(::core::default::Default::default())
    }
    fn __repr__(&self) -> ::std::string::String {
        ::std::format!("{:?}", self.0)
    }
    fn __eq__(&self, other: &Self) -> bool {
        self.0 == other.0
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
#[::pyo3::pyclass(name = "GetTagAssignmentRequest", from_py_object)]
#[derive(Clone, Debug)]
pub struct PyGetTagAssignmentRequest(pub super::tags::v1::GetTagAssignmentRequest);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyGetTagAssignmentRequest {
    #[new]
    #[pyo3(signature = (entity_type = None, entity_name = None, tag_key = None))]
    fn new(
        entity_type: ::core::option::Option<::std::string::String>,
        entity_name: ::core::option::Option<::std::string::String>,
        tag_key: ::core::option::Option<::std::string::String>,
    ) -> Self {
        let mut inner = <super::tags::v1::GetTagAssignmentRequest as ::core::default::Default>::default();
        if let ::core::option::Option::Some(value) = entity_type {
            inner.entity_type = value;
        }
        if let ::core::option::Option::Some(value) = entity_name {
            inner.entity_name = value;
        }
        if let ::core::option::Option::Some(value) = tag_key {
            inner.tag_key = value;
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
    #[setter(entity_type)]
    fn set_entity_type(&mut self, value: ::std::string::String) {
        self.0.entity_type = value;
    }
    #[setter(entity_name)]
    fn set_entity_name(&mut self, value: ::std::string::String) {
        self.0.entity_name = value;
    }
    #[setter(tag_key)]
    fn set_tag_key(&mut self, value: ::std::string::String) {
        self.0.tag_key = value;
    }
    fn __repr__(&self) -> ::std::string::String {
        ::std::format!("{:?}", self.0)
    }
    fn __eq__(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl ::core::convert::From<super::tags::v1::GetTagAssignmentRequest>
for PyGetTagAssignmentRequest {
    fn from(value: super::tags::v1::GetTagAssignmentRequest) -> Self {
        Self(value)
    }
}
impl ::core::convert::From<PyGetTagAssignmentRequest>
for super::tags::v1::GetTagAssignmentRequest {
    fn from(value: PyGetTagAssignmentRequest) -> Self {
        value.0
    }
}
#[::pyo3::pyclass(name = "ListTagAssignmentsRequest", from_py_object)]
#[derive(Clone, Debug)]
pub struct PyListTagAssignmentsRequest(pub super::tags::v1::ListTagAssignmentsRequest);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyListTagAssignmentsRequest {
    #[new]
    #[pyo3(
        signature = (
            entity_type = None,
            entity_name = None,
            max_results = None,
            page_token = None
        )
    )]
    fn new(
        entity_type: ::core::option::Option<::std::string::String>,
        entity_name: ::core::option::Option<::std::string::String>,
        max_results: ::core::option::Option<i32>,
        page_token: ::core::option::Option<::std::string::String>,
    ) -> Self {
        let mut inner = <super::tags::v1::ListTagAssignmentsRequest as ::core::default::Default>::default();
        if let ::core::option::Option::Some(value) = entity_type {
            inner.entity_type = value;
        }
        if let ::core::option::Option::Some(value) = entity_name {
            inner.entity_name = value;
        }
        if let ::core::option::Option::Some(value) = max_results {
            inner.max_results = value;
        }
        if let ::core::option::Option::Some(value) = page_token {
            inner.page_token = value;
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
    fn max_results(&self) -> i32 {
        self.0.max_results
    }
    #[getter]
    fn page_token(&self) -> ::std::string::String {
        self.0.page_token.clone()
    }
    #[setter(entity_type)]
    fn set_entity_type(&mut self, value: ::std::string::String) {
        self.0.entity_type = value;
    }
    #[setter(entity_name)]
    fn set_entity_name(&mut self, value: ::std::string::String) {
        self.0.entity_name = value;
    }
    #[setter(max_results)]
    fn set_max_results(&mut self, value: i32) {
        self.0.max_results = value;
    }
    #[setter(page_token)]
    fn set_page_token(&mut self, value: ::std::string::String) {
        self.0.page_token = value;
    }
    fn __repr__(&self) -> ::std::string::String {
        ::std::format!("{:?}", self.0)
    }
    fn __eq__(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl ::core::convert::From<super::tags::v1::ListTagAssignmentsRequest>
for PyListTagAssignmentsRequest {
    fn from(value: super::tags::v1::ListTagAssignmentsRequest) -> Self {
        Self(value)
    }
}
impl ::core::convert::From<PyListTagAssignmentsRequest>
for super::tags::v1::ListTagAssignmentsRequest {
    fn from(value: PyListTagAssignmentsRequest) -> Self {
        value.0
    }
}
#[::pyo3::pyclass(name = "ListTagAssignmentsResponse", from_py_object)]
#[derive(Clone, Debug)]
pub struct PyListTagAssignmentsResponse(pub super::tags::v1::ListTagAssignmentsResponse);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyListTagAssignmentsResponse {
    #[new]
    #[pyo3(signature = (tag_assignments = None, next_page_token = None))]
    fn new(
        tag_assignments: ::core::option::Option<::std::vec::Vec<PyTagAssignment>>,
        next_page_token: ::core::option::Option<::std::string::String>,
    ) -> Self {
        let mut inner = <super::tags::v1::ListTagAssignmentsResponse as ::core::default::Default>::default();
        if let ::core::option::Option::Some(value) = tag_assignments {
            inner.tag_assignments = value
                .into_iter()
                .map(::core::convert::Into::into)
                .collect();
        }
        if let ::core::option::Option::Some(value) = next_page_token {
            inner.next_page_token = value;
        }
        Self(inner)
    }
    #[getter]
    fn tag_assignments(&self) -> ::std::vec::Vec<PyTagAssignment> {
        self.0.tag_assignments.iter().cloned().map(PyTagAssignment::from).collect()
    }
    #[getter]
    fn next_page_token(&self) -> ::std::string::String {
        self.0.next_page_token.clone()
    }
    #[setter(tag_assignments)]
    fn set_tag_assignments(&mut self, value: ::std::vec::Vec<PyTagAssignment>) {
        self.0.tag_assignments = value
            .into_iter()
            .map(::core::convert::Into::into)
            .collect();
    }
    #[setter(next_page_token)]
    fn set_next_page_token(&mut self, value: ::std::string::String) {
        self.0.next_page_token = value;
    }
    fn __repr__(&self) -> ::std::string::String {
        ::std::format!("{:?}", self.0)
    }
    fn __eq__(&self, other: &Self) -> bool {
        self.0 == other.0
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
#[::pyo3::pyclass(name = "TagAssignment", from_py_object)]
#[derive(Clone, Debug)]
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
    #[setter(entity_type)]
    fn set_entity_type(&mut self, value: ::std::string::String) {
        self.0.entity_type = value;
    }
    #[setter(entity_name)]
    fn set_entity_name(&mut self, value: ::std::string::String) {
        self.0.entity_name = value;
    }
    #[setter(tag_key)]
    fn set_tag_key(&mut self, value: ::std::string::String) {
        self.0.tag_key = value;
    }
    #[setter(tag_value)]
    fn set_tag_value(&mut self, value: ::std::string::String) {
        self.0.tag_value = value;
    }
    fn __repr__(&self) -> ::std::string::String {
        ::std::format!("{:?}", self.0)
    }
    fn __eq__(&self, other: &Self) -> bool {
        self.0 == other.0
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
#[::pyo3::pyclass(name = "TouchTagAssignmentRequest", from_py_object)]
#[derive(Clone, Debug)]
pub struct PyTouchTagAssignmentRequest(pub super::tags::v1::TouchTagAssignmentRequest);
#[allow(clippy::too_many_arguments, clippy::useless_conversion)]
#[::pyo3::pymethods]
impl PyTouchTagAssignmentRequest {
    #[new]
    #[pyo3(signature = (entity_type = None, entity_name = None, tag_key = None))]
    fn new(
        entity_type: ::core::option::Option<::std::string::String>,
        entity_name: ::core::option::Option<::std::string::String>,
        tag_key: ::core::option::Option<::std::string::String>,
    ) -> Self {
        let mut inner = <super::tags::v1::TouchTagAssignmentRequest as ::core::default::Default>::default();
        if let ::core::option::Option::Some(value) = entity_type {
            inner.entity_type = value;
        }
        if let ::core::option::Option::Some(value) = entity_name {
            inner.entity_name = value;
        }
        if let ::core::option::Option::Some(value) = tag_key {
            inner.tag_key = value;
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
    #[setter(entity_type)]
    fn set_entity_type(&mut self, value: ::std::string::String) {
        self.0.entity_type = value;
    }
    #[setter(entity_name)]
    fn set_entity_name(&mut self, value: ::std::string::String) {
        self.0.entity_name = value;
    }
    #[setter(tag_key)]
    fn set_tag_key(&mut self, value: ::std::string::String) {
        self.0.tag_key = value;
    }
    fn __repr__(&self) -> ::std::string::String {
        ::std::format!("{:?}", self.0)
    }
    fn __eq__(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl ::core::convert::From<super::tags::v1::TouchTagAssignmentRequest>
for PyTouchTagAssignmentRequest {
    fn from(value: super::tags::v1::TouchTagAssignmentRequest) -> Self {
        Self(value)
    }
}
impl ::core::convert::From<PyTouchTagAssignmentRequest>
for super::tags::v1::TouchTagAssignmentRequest {
    fn from(value: PyTouchTagAssignmentRequest) -> Self {
        value.0
    }
}
