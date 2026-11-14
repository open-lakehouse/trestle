// @generated — do not edit by hand.
//! The authorization vocabulary of the API.
//!
//! Every RPC is an [`Operation`], named after it, and every request an [`Authorizable`]
//! that names its operation and the [`Securable`] it acts on. A policy decides whether a
//! caller may perform an operation on a securable; the Cedar actions of the operations
//! are in the sibling `authz.cedarschema`.

use example_common::models::ObjectLabel;

/// An operation of the API: one per RPC, named after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Operation {
    /// Lists the catalogs.
    ListCatalogs,
    /// Creates a catalog.
    CreateCatalog,
    /// Gets a catalog.
    GetCatalog,
    CreateDraft,
    GetDraft,
    GetPermissions,
    GetTablePermissions,
    CheckTable,
    ListSchemas,
    CreateSchema,
    GetSchema,
    ListTables,
    ListTableSummaries,
    CreateTable,
    GetTable,
    GetTableVersion,
}

impl Operation {
    /// Every operation, in declaration order.
    pub const ALL: &'static [Operation] = &[
        Operation::ListCatalogs,
        Operation::CreateCatalog,
        Operation::GetCatalog,
        Operation::CreateDraft,
        Operation::GetDraft,
        Operation::GetPermissions,
        Operation::GetTablePermissions,
        Operation::CheckTable,
        Operation::ListSchemas,
        Operation::CreateSchema,
        Operation::GetSchema,
        Operation::ListTables,
        Operation::ListTableSummaries,
        Operation::CreateTable,
        Operation::GetTable,
        Operation::GetTableVersion,
    ];
    /// The name of the operation: the RPC name, which is also its Cedar action id.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Operation::ListCatalogs => "ListCatalogs",
            Operation::CreateCatalog => "CreateCatalog",
            Operation::GetCatalog => "GetCatalog",
            Operation::CreateDraft => "CreateDraft",
            Operation::GetDraft => "GetDraft",
            Operation::GetPermissions => "GetPermissions",
            Operation::GetTablePermissions => "GetTablePermissions",
            Operation::CheckTable => "CheckTable",
            Operation::ListSchemas => "ListSchemas",
            Operation::CreateSchema => "CreateSchema",
            Operation::GetSchema => "GetSchema",
            Operation::ListTables => "ListTables",
            Operation::ListTableSummaries => "ListTableSummaries",
            Operation::CreateTable => "CreateTable",
            Operation::GetTable => "GetTable",
            Operation::GetTableVersion => "GetTableVersion",
        }
    }

    /// The kind of securable the operation is asked on.
    pub const fn securable_kind(&self) -> SecurableKind {
        match self {
            Operation::ListCatalogs => SecurableKind::Root,
            Operation::CreateCatalog => SecurableKind::Resource(ObjectLabel::Catalog),
            Operation::GetCatalog => SecurableKind::Resource(ObjectLabel::Catalog),
            Operation::CreateDraft => SecurableKind::Resource(ObjectLabel::Draft),
            Operation::GetDraft => SecurableKind::Resource(ObjectLabel::Draft),
            Operation::GetPermissions => SecurableKind::Any,
            Operation::GetTablePermissions => SecurableKind::Resource(ObjectLabel::Table),
            Operation::CheckTable => SecurableKind::Resource(ObjectLabel::Table),
            Operation::ListSchemas => SecurableKind::Resource(ObjectLabel::Catalog),
            Operation::CreateSchema => SecurableKind::Resource(ObjectLabel::Schema),
            Operation::GetSchema => SecurableKind::Resource(ObjectLabel::Schema),
            Operation::ListTables => SecurableKind::Resource(ObjectLabel::Schema),
            Operation::ListTableSummaries => {
                SecurableKind::Resource(ObjectLabel::Catalog)
            }
            Operation::CreateTable => SecurableKind::Resource(ObjectLabel::Table),
            Operation::GetTable => SecurableKind::Resource(ObjectLabel::Table),
            Operation::GetTableVersion => SecurableKind::Resource(ObjectLabel::Table),
        }
    }

    /// The operation that reads one resource of `label` (the `Get` RPC of its service),
    /// if there is one. List results can be filtered with it.
    #[allow(unreachable_patterns)]
    pub const fn get_of(label: ObjectLabel) -> Option<Operation> {
        match label {
            ObjectLabel::Catalog => Some(Operation::GetCatalog),
            ObjectLabel::Draft => Some(Operation::GetDraft),
            ObjectLabel::Schema => Some(Operation::GetSchema),
            ObjectLabel::Table => Some(Operation::GetTable),
            _ => None,
        }
    }
}

impl ::std::fmt::Display for Operation {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl ::std::str::FromStr for Operation {
    type Err = UnknownOperation;
    fn from_str(s: &str) -> ::std::result::Result<Self, Self::Err> {
        match s {
            "ListCatalogs" => Ok(Operation::ListCatalogs),
            "CreateCatalog" => Ok(Operation::CreateCatalog),
            "GetCatalog" => Ok(Operation::GetCatalog),
            "CreateDraft" => Ok(Operation::CreateDraft),
            "GetDraft" => Ok(Operation::GetDraft),
            "GetPermissions" => Ok(Operation::GetPermissions),
            "GetTablePermissions" => Ok(Operation::GetTablePermissions),
            "CheckTable" => Ok(Operation::CheckTable),
            "ListSchemas" => Ok(Operation::ListSchemas),
            "CreateSchema" => Ok(Operation::CreateSchema),
            "GetSchema" => Ok(Operation::GetSchema),
            "ListTables" => Ok(Operation::ListTables),
            "ListTableSummaries" => Ok(Operation::ListTableSummaries),
            "CreateTable" => Ok(Operation::CreateTable),
            "GetTable" => Ok(Operation::GetTable),
            "GetTableVersion" => Ok(Operation::GetTableVersion),
            other => Err(UnknownOperation(other.to_string())),
        }
    }
}

/// The kind of resource an [`Operation`] acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SecurableKind {
    /// The service root, e.g. for a list of top-level resources.
    Root,
    /// A resource of this kind.
    Resource(ObjectLabel),
    /// Any resource, as the request decides at run time (e.g. a request naming the type
    /// of the resource it acts on).
    Any,
}

/// The error of parsing a name that is not an [`Operation`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownOperation(pub String);
impl ::std::fmt::Display for UnknownOperation {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        write!(f, "unknown operation `{}`", self.0)
    }
}

impl ::std::error::Error for UnknownOperation {}
/// The resource a request acts on.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Securable {
    /// The kind of resource, or `None` for the service root.
    pub label: Option<ObjectLabel>,
    /// The parts of the resource's name, outermost first (e.g. a parent's name, then
    /// the resource's own). Empty for the root. A list names the container of the
    /// resources it lists; a create names the new resource.
    ///
    /// A request that leaves a name field empty gives an empty part, which names no
    /// resource: deny it rather than reading the shorter name as a container.
    pub name: Vec<String>,
}

impl Securable {
    /// The resource of kind `label` named `name`.
    pub fn new(label: ObjectLabel, name: Vec<String>) -> Self {
        Self { label: Some(label), name }
    }

    /// The service root.
    pub fn root() -> Self {
        Self {
            label: None,
            name: Vec::new(),
        }
    }
}

/// A request that names its operation and the securable it acts on.
pub trait Authorizable: Send + Sync {
    /// The operation the request performs.
    fn operation(&self) -> Operation;
    /// The resource the request acts on.
    fn securable(&self) -> Securable;
}
