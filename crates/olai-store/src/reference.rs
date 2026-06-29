use uuid::Uuid;

use crate::ResourceName;

/// A reference to a resource, by id, by name, or unspecified.
///
/// A resource can be addressed two ways: by its stable [`Uuid`] or by its
/// human-readable [`ResourceName`]. [`Undefined`] is a third state for when no
/// particular resource is meant (a wildcard), used chiefly in policy checks.
///
/// [`Undefined`]: ResourceRef::Undefined
#[derive(Debug, Clone, PartialEq, Hash, Eq)]
pub enum ResourceRef {
    /// References a resource by its stable unique identifier.
    Uuid(Uuid),
    /// References a resource by its hierarchical [`ResourceName`].
    Name(ResourceName),
    /// Not referencing a specific resource.
    ///
    /// This is used to represent a wildcard in a policy
    /// which can be useful to check if a user can create
    /// or manage resources at a specific level.
    Undefined,
}

impl ResourceRef {
    /// Returns `true` if this reference is [`ResourceRef::Undefined`] (a wildcard).
    pub fn is_undefined(&self) -> bool {
        matches!(self, Self::Undefined)
    }

    /// Constructs a [`ResourceRef::Name`] from anything convertible into a
    /// [`ResourceName`].
    pub fn name(name: impl Into<ResourceName>) -> Self {
        Self::Name(name.into())
    }
}

impl std::fmt::Display for ResourceRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Uuid(u) => write!(f, "{}", u.hyphenated()),
            Self::Name(name) => {
                write!(f, "{name}")
            }
            Self::Undefined => write!(f, "*"),
        }
    }
}

impl From<Uuid> for ResourceRef {
    fn from(val: Uuid) -> Self {
        Self::Uuid(val)
    }
}

impl From<&Uuid> for ResourceRef {
    fn from(val: &Uuid) -> Self {
        Self::Uuid(*val)
    }
}

impl From<ResourceName> for ResourceRef {
    fn from(val: ResourceName) -> Self {
        Self::Name(val)
    }
}
