// @generated — do not edit by hand.
/// All resource types managed by the service.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, Debug, PartialEq)]
pub enum Resource {
    Catalog(super::catalog::v1::Catalog),
}
/// Discriminant label for each resource type.
#[derive(
    ::strum::AsRefStr,
    ::strum::Display,
    ::strum::EnumIter,
    ::strum::EnumString,
    ::serde::Serialize,
    ::serde::Deserialize,
    Hash,
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
)]
#[strum(serialize_all = "snake_case", ascii_case_insensitive)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "sqlx", derive(::sqlx::Type))]
#[cfg_attr(
    feature = "sqlx",
    sqlx(type_name = "object_label", rename_all = "snake_case")
)]
pub enum ObjectLabel {
    Catalog,
}
impl Resource {
    /// Return the discriminant label for this resource.
    pub fn resource_label(&self) -> &ObjectLabel {
        match self {
            Resource::Catalog(_) => &ObjectLabel::Catalog,
        }
    }
}
impl From<super::catalog::v1::Catalog> for Resource {
    fn from(v: super::catalog::v1::Catalog) -> Self {
        Resource::Catalog(v)
    }
}
impl TryFrom<Resource> for super::catalog::v1::Catalog {
    type Error = crate::Error;
    fn try_from(r: Resource) -> Result<Self, Self::Error> {
        match r {
            Resource::Catalog(v) => Ok(v),
        }
    }
}
impl ::olai_store::Label for ObjectLabel {
    fn as_str(&self) -> &str {
        self.as_ref()
    }
}
/// Static resource type descriptors derived from proto annotations.
///
/// Each entry describes a resource type's fields (with roles: data, identifier,
/// sensitive, managed), hierarchical name components, and parent relationship.
///
/// Use `ResourceRegistry::from_static` to build a runtime registry from this data.
pub static RESOURCE_DESCRIPTORS: &[::olai_store::ResourceTypeDescriptor<ObjectLabel>] = &[
    ::olai_store::ResourceTypeDescriptor {
        label: ObjectLabel::Catalog,
        fields: &[
            ::olai_store::ResourceFieldDescriptor {
                name: "name",
                role: ::olai_store::FieldRole::Data,
            },
            ::olai_store::ResourceFieldDescriptor {
                name: "comment",
                role: ::olai_store::FieldRole::Data,
            },
            ::olai_store::ResourceFieldDescriptor {
                name: "catalog_type",
                role: ::olai_store::FieldRole::Data,
            },
            ::olai_store::ResourceFieldDescriptor {
                name: "properties",
                role: ::olai_store::FieldRole::Data,
            },
            ::olai_store::ResourceFieldDescriptor {
                name: "storage_config",
                role: ::olai_store::FieldRole::Data,
            },
            ::olai_store::ResourceFieldDescriptor {
                name: "created_at",
                role: ::olai_store::FieldRole::Managed,
            },
        ],
        path_names: &["name"],
        parent_label: None,
    },
];
