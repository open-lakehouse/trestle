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
//!   (data / identifier / managed / sensitive) from a [`ResourceRegistry`], sealing
//!   sensitive values with an optional `EnvelopeEncryptor` (see the `encryption` feature).
//!
//! **Note:** this store favours simplicity over features and performance. It's a
//! good default for bootstrapping Trestle projects, prototypes, and demos, but is
//! not intended for serious production workloads — back these traits with your
//! own production-grade engine for those.
//!
//! # Examples
//!
//! Define a resource taxonomy by implementing [`Label`] for an enum. The store
//! traits are generic over this type:
//!
//! ```
//! use std::fmt;
//! use std::str::FromStr;
//!
//! use olai_store::Label;
//!
//! #[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
//! enum Kind {
//!     Catalog,
//!     Schema,
//!     Table,
//! }
//!
//! impl Kind {
//!     fn as_str(&self) -> &'static str {
//!         match self {
//!             Kind::Catalog => "catalog",
//!             Kind::Schema => "schema",
//!             Kind::Table => "table",
//!         }
//!     }
//! }
//!
//! impl fmt::Display for Kind {
//!     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
//!         f.write_str(self.as_str())
//!     }
//! }
//!
//! impl FromStr for Kind {
//!     type Err = String;
//!
//!     fn from_str(s: &str) -> Result<Self, Self::Err> {
//!         match s {
//!             "catalog" => Ok(Kind::Catalog),
//!             "schema" => Ok(Kind::Schema),
//!             "table" => Ok(Kind::Table),
//!             other => Err(format!("unknown kind: {other}")),
//!         }
//!     }
//! }
//!
//! impl Label for Kind {
//!     fn as_str(&self) -> &str {
//!         Kind::as_str(self)
//!     }
//! }
//!
//! assert_eq!(Kind::Table.as_str(), "table");
//! assert_eq!("schema".parse::<Kind>(), Ok(Kind::Schema));
//! ```
//!
//! [Trestle]: https://github.com/open-lakehouse/trestle

pub mod backend;
pub mod conformance;
#[cfg(feature = "encryption")]
pub mod encryption;
pub mod error;
pub mod label;
pub mod managed;
pub mod name;
pub mod object;
pub mod reference;
pub mod registry;
pub mod store;

// Re-exports for convenience.
pub use backend::mem::InMemoryStore;
#[cfg(feature = "sqlite")]
pub use backend::sql::{SqlStore, migrate as migrate_sql, migrator as sql_migrator};
#[cfg(feature = "encryption")]
pub use encryption::{EnvelopeEncryptor, KekId, KeyProvider, LocalKeyProvider};
pub use error::{Error, Result};
pub use label::Label;
pub use managed::ManagedObjectStore;
pub use name::{EMPTY_RESOURCE_NAME, ResourceName};
pub use object::{Association, Object};
pub use reference::ResourceRef;
pub use registry::{FieldRole, ResourceFieldDescriptor, ResourceRegistry, ResourceTypeDescriptor};
pub use store::{
    AssociationStore, AssociationStoreReader, ObjectStore, ObjectStoreReader, Precondition,
    StoreExec, StoreTx, Transactional,
};
