// @generated — do not edit by hand.
/// All resource types managed by the service.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, Debug, PartialEq)]
pub enum Resource {
    Catalog(super::catalog::v1::Catalog),
    Schema(super::schemas::v1::Schema),
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
    Schema,
}
impl Resource {
    /// Return the discriminant label for this resource.
    pub fn resource_label(&self) -> &ObjectLabel {
        match self {
            Resource::Catalog(_) => &ObjectLabel::Catalog,
            Resource::Schema(_) => &ObjectLabel::Schema,
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
            _ => {
                Err(
                    <crate::Error>::generic(
                        concat!("Resource is not a ", stringify!(Catalog)),
                    ),
                )
            }
        }
    }
}
impl From<super::schemas::v1::Schema> for Resource {
    fn from(v: super::schemas::v1::Schema) -> Self {
        Resource::Schema(v)
    }
}
impl TryFrom<Resource> for super::schemas::v1::Schema {
    type Error = crate::Error;
    fn try_from(r: Resource) -> Result<Self, Self::Error> {
        match r {
            Resource::Schema(v) => Ok(v),
            _ => {
                Err(
                    <crate::Error>::generic(
                        concat!("Resource is not a ", stringify!(Schema)),
                    ),
                )
            }
        }
    }
}
use crate::Error;
use crate::models::object::Object;
use crate::models::resources::{ResourceExt, ResourceIdent, ResourceName, ResourceRef};
impl TryFrom<Object> for super::catalog::v1::Catalog {
    type Error = Error;
    fn try_from(object: Object) -> Result<Self, Self::Error> {
        let props = object
            .properties
            .ok_or_else(|| Error::generic("expected properties"))?;
        let mut res: super::catalog::v1::Catalog = ::serde_json::from_value(props)?;
        res.name = object.id.hyphenated().to_string();
        Ok(res)
    }
}
impl TryFrom<super::catalog::v1::Catalog> for Object {
    type Error = Error;
    fn try_from(obj: super::catalog::v1::Catalog) -> Result<Self, Self::Error> {
        let id = ::uuid::Uuid::parse_str(&obj.name)
            .unwrap_or_else(|_| ::uuid::Uuid::nil());
        Ok(Object {
            id,
            name: obj.resource_name(),
            label: ObjectLabel::Catalog,
            properties: Some(::serde_json::to_value(obj)?),
            version: 0,
            updated_at: None,
            created_at: chrono::Utc::now(),
        })
    }
}
impl ResourceExt for super::catalog::v1::Catalog {
    fn resource_name(&self) -> ResourceName {
        ResourceName::new([&self.name])
    }
    fn resource_ref(&self) -> ResourceRef {
        ::uuid::Uuid::parse_str(&self.name)
            .ok()
            .map(ResourceRef::Uuid)
            .unwrap_or_else(|| ResourceRef::Name(self.resource_name()))
    }
    fn resource_ident(&self) -> ResourceIdent {
        (ObjectLabel::Catalog).to_ident(self.resource_ref())
    }
}
impl super::catalog::v1::Catalog {
    /// Returns the fully-qualified dot-separated name computed from component fields.
    pub fn qualified_name(&self) -> String {
        self.name.clone()
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
                role: ::olai_store::FieldRole::Identifier,
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
    ::olai_store::ResourceTypeDescriptor {
        label: ObjectLabel::Schema,
        fields: &[
            ::olai_store::ResourceFieldDescriptor {
                name: "full_name",
                role: ::olai_store::FieldRole::Data,
            },
            ::olai_store::ResourceFieldDescriptor {
                name: "comment",
                role: ::olai_store::FieldRole::Data,
            },
            ::olai_store::ResourceFieldDescriptor {
                name: "schema_type",
                role: ::olai_store::FieldRole::Data,
            },
            ::olai_store::ResourceFieldDescriptor {
                name: "created_at",
                role: ::olai_store::FieldRole::Managed,
            },
        ],
        path_names: &["catalog_name", "name"],
        parent_label: Some(ObjectLabel::Catalog),
    },
];
