#![cfg_attr(docsrs, feature(doc_cfg))]
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
//!   — the async read/write storage traits (with `*Reader` read-only
//!   counterparts). The bundled [`InMemoryStore`] and `SqlStore` backends
//!   implement them; they are a **taxonomy-blind blob layer** — they persist
//!   already-shaped properties plus an opaque sensitive blob and know nothing of
//!   field roles.
//! - [`ManagedObjectStore`] — **the store you reach for.** It wraps a backend and
//!   a [`ResourceRegistry`] to enforce field roles (data / identifier / managed /
//!   sensitive): stripping store-owned fields, injecting them back on read, and
//!   sealing/redacting sensitive fields. Encryption is optional — build it with
//!   [`new`](ManagedObjectStore::new) for a store with no sensitive fields, or
//!   `with_encryptor` (the `encryption`
//!   feature) to seal them. Writing a resource that *has* sensitive fields through
//!   a store with no encryptor is a hard error, never a silent drop.
//!
//! Most callers construct a `ManagedObjectStore` and use it through the
//! `ObjectStore` trait; the raw backends are the pluggable layer underneath.
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
pub mod filter;
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
pub use filter::{CompareOp, FieldPath, Filter, Predicate};
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
