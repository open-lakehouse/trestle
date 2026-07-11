//! Shared conformance battery for store backends.
//!
//! Populated in a later step. It provides a set of `async` assertions
//! parameterized over any implementation of the store traits ([`ObjectStore`],
//! [`AssociationStore`], and [`Transactional`]), covering CAS conflicts, rename
//! identity and association preservation, transaction atomicity, and
//! inverse-edge consistency. The battery is run against
//! [`InMemoryStore`](crate::InMemoryStore) and the sqlx `SqlStore`.
//!
//! [`ObjectStore`]: crate::ObjectStore
//! [`AssociationStore`]: crate::AssociationStore
//! [`Transactional`]: crate::Transactional
