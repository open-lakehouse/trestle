//! Generic, TAO-inspired resource store for typed objects and associations — the
//! async storage layer for services built with the [Trestle] framework.
//!
//! Core abstractions for a graph-based store, generic over `L: [Label]` (a
//! type-safe resource-type discriminant, typically generated from
//! `google.api.resource` annotations):
//!
//! - [`Object<L>`][Object] — a resource node: UUID, label, hierarchical
//!   [`ResourceName`], and a JSON properties blob.
//! - [`Association<L>`][Association] — a directed edge between two objects.
//! - [`ObjectStore<L>`][ObjectStore] / [`AssociationStore<L>`][AssociationStore]
//!   — async read/write traits (with `*Reader` read-only counterparts).
//! - [`ManagedObjectStore`] — wraps an `ObjectStore` and enforces field roles
//!   (data / identifier / managed / sensitive) from a [`ResourceRegistry`].
//! - [`SecretManager`] — encrypted storage for sensitive field values.
//!
//! **Note:** this store favours simplicity over features and performance. It's a
//! good default for bootstrapping Trestle projects, prototypes, and demos, but is
//! not intended for serious production workloads — back these traits with your
//! own production-grade engine for those.
//!
//! [Trestle]: https://github.com/open-lakehouse/trestle

pub mod error;
pub mod label;
pub mod managed;
pub mod name;
pub mod object;
pub mod reference;
pub mod registry;
pub mod secrets;
pub mod store;

// Re-exports for convenience.
pub use error::{Error, Result};
pub use label::Label;
pub use managed::{ManagedObjectStore, NoSecrets};
pub use name::{EMPTY_RESOURCE_NAME, ResourceName};
pub use object::{Association, Object};
pub use reference::ResourceRef;
pub use registry::{FieldRole, ResourceFieldDescriptor, ResourceRegistry, ResourceTypeDescriptor};
pub use secrets::{ProvidesSecretManager, SecretManager};
pub use store::{AssociationStore, AssociationStoreReader, ObjectStore, ObjectStoreReader};
