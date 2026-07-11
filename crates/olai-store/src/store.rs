use std::sync::Arc;

use uuid::Uuid;

use crate::Result;
use crate::label::Label;
use crate::name::ResourceName;
use crate::object::{Association, Object};

/// A precondition guarding a mutating object operation (optimistic concurrency).
///
/// Modelled as an extensible value object (cf. the `object_store` crate's
/// `PutMode` and Google [AIP-154]) so new precondition kinds can be added
/// without changing method signatures. A mismatch yields
/// [`Error::Conflict`](crate::Error::Conflict) — never
/// [`Error::NotFound`](crate::Error::NotFound), since the object may exist at a
/// different version.
///
/// [AIP-154]: https://google.aip.dev/154
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Precondition {
    /// No check — the operation applies unconditionally.
    #[default]
    Any,
    /// Compare-and-swap: apply only if the stored [`version`](Object::version)
    /// equals this value, otherwise return
    /// [`Error::Conflict`](crate::Error::Conflict).
    Version(u64),
}

impl Precondition {
    /// Check `self` against an object's current `version`, returning
    /// [`Error::Conflict`](crate::Error::Conflict) on a
    /// [`Precondition::Version`] mismatch.
    ///
    /// A helper for store backends implementing compare-and-swap:
    ///
    /// ```
    /// use olai_store::Precondition;
    ///
    /// assert!(Precondition::Any.check(7).is_ok());
    /// assert!(Precondition::Version(7).check(7).is_ok());
    /// assert!(Precondition::Version(6).check(7).is_err());
    /// ```
    pub fn check(&self, current_version: u64) -> Result<()> {
        match self {
            Precondition::Any => Ok(()),
            Precondition::Version(expected) if *expected == current_version => Ok(()),
            Precondition::Version(_) => Err(crate::Error::Conflict),
        }
    }
}

/// Read-only interface for the object store.
#[async_trait::async_trait]
pub trait ObjectStoreReader<L: Label>: Send + Sync + 'static {
    /// Get an object by its UUID.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotFound`](crate::Error::NotFound) if no object with `id` exists.
    async fn get(&self, id: &Uuid) -> Result<Object<L>>;

    /// Get an object by its label and name.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotFound`](crate::Error::NotFound) if no object with the given `label` and `name`
    /// exists.
    async fn get_by_name(&self, label: L, name: &ResourceName) -> Result<Object<L>>;

    /// List objects of a given label, optionally scoped to a namespace prefix.
    ///
    /// Returns the matching objects and an optional continuation token. Results
    /// are returned in a stable order so that paging is deterministic. At most
    /// `max_results` objects are returned per call; when more remain, the returned
    /// token is `Some` and should be passed back as `page_token` to fetch the next
    /// page. A returned token of `None` indicates the final page. `page_token`
    /// must be a token previously produced by this method on the same query.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidArgument`](crate::Error::InvalidArgument) if `page_token` is not a valid token for
    /// this query.
    async fn list(
        &self,
        label: L,
        namespace: Option<&ResourceName>,
        max_results: Option<usize>,
        page_token: Option<String>,
    ) -> Result<(Vec<Object<L>>, Option<String>)>;
}

/// Read-write interface for the object store.
#[async_trait::async_trait]
pub trait ObjectStore<L: Label>: ObjectStoreReader<L> + Send + Sync + 'static {
    /// Create a new object. The store generates `created_at` and `updated_at`.
    ///
    /// `id` lets the caller pre-allocate the object's id (e.g. a managed table
    /// adopting the id reserved by its staging reservation, or a managed volume
    /// embedding the id in its storage path); pass `None` to have the store
    /// generate a time-ordered id.
    ///
    /// # Errors
    ///
    /// - [`Error::AlreadyExists`](crate::Error::AlreadyExists) if an object with the same `label` and `name`,
    ///   or with the supplied `id`, already exists.
    /// - [`Error::InvalidArgument`](crate::Error::InvalidArgument) if `name` or `properties` are malformed.
    async fn create(
        &self,
        label: L,
        name: &ResourceName,
        properties: Option<serde_json::Value>,
        id: Option<Uuid>,
    ) -> Result<Object<L>>;

    /// Update an existing object's properties.
    ///
    /// The returned object carries the incremented [`version`](Object::version).
    /// Pass a [`Precondition::Version`] to make this a compare-and-swap and close
    /// the read-modify-write race; [`Precondition::Any`] overwrites
    /// unconditionally.
    ///
    /// # Errors
    ///
    /// - [`Error::NotFound`](crate::Error::NotFound) if no object with `id` exists.
    /// - [`Error::Conflict`](crate::Error::Conflict) if `precondition` is
    ///   [`Precondition::Version`] and the stored version no longer matches.
    /// - [`Error::InvalidArgument`](crate::Error::InvalidArgument) if `properties` are malformed.
    async fn update(
        &self,
        id: &Uuid,
        properties: Option<serde_json::Value>,
        precondition: Precondition,
    ) -> Result<Object<L>>;

    /// Rename (or move) an object to a new [`ResourceName`], preserving its
    /// `id`, associations, and any secrets.
    ///
    /// The new name may change the object's namespace (a cross-subtree move) or
    /// only its leaf segment; the store layer permits either, and higher layers
    /// gate policy. The returned object carries the incremented
    /// [`version`](Object::version).
    ///
    /// # Errors
    ///
    /// - [`Error::NotFound`](crate::Error::NotFound) if no object with `id` exists.
    /// - [`Error::AlreadyExists`](crate::Error::AlreadyExists) if an object with
    ///   the same label and `new_name` already exists.
    /// - [`Error::Conflict`](crate::Error::Conflict) if `precondition` is
    ///   [`Precondition::Version`] and the stored version no longer matches.
    /// - [`Error::InvalidArgument`](crate::Error::InvalidArgument) if `new_name` is malformed.
    async fn rename(
        &self,
        id: &Uuid,
        new_name: &ResourceName,
        precondition: Precondition,
    ) -> Result<Object<L>>;

    /// Delete an object and all its associations.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotFound`](crate::Error::NotFound) if no object with `id` exists.
    async fn delete(&self, id: &Uuid) -> Result<()>;
}

/// Read-only interface for the association (edge) store.
#[async_trait::async_trait]
pub trait AssociationStoreReader<L: Label>: Send + Sync + 'static {
    /// List associations from a given source object with a specific edge label.
    ///
    /// Optionally filter by the target object's label. Results are returned in a
    /// stable order; at most `max_results` associations are returned per call, and
    /// when more remain the returned continuation token is `Some` and should be
    /// passed back as `page_token` to fetch the next page (a `None` token marks the
    /// final page). `page_token` must be a token previously produced by this method
    /// on the same query.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidArgument`](crate::Error::InvalidArgument) if `page_token` is not a valid token for
    /// this query.
    async fn list(
        &self,
        from_id: Uuid,
        label: &str,
        target_label: Option<L>,
        max_results: Option<usize>,
        page_token: Option<String>,
    ) -> Result<(Vec<Association<L>>, Option<String>)>;
}

/// Read-write interface for the association (edge) store.
///
/// Associations are directed edges between objects: an edge with `label` runs
/// *from* a source object *to* a target object. The *inverse edge* is the edge
/// pointing the other way — from the target back to the source — under the edge
/// label's paired inverse label (for example, a `parent_of` edge has the inverse
/// `child_of`). Maintaining the inverse edge lets the graph be traversed in both
/// directions: listing the source's outgoing edges and the target's incoming
/// edges both stay consistent. Implementations should create and remove the
/// inverse edge alongside the primary edge whenever the edge label has one.
#[async_trait::async_trait]
pub trait AssociationStore<L: Label>: AssociationStoreReader<L> + Send + Sync + 'static {
    /// Add an association between two objects.
    ///
    /// The implementation should also create the inverse association if the
    /// edge label has one.
    ///
    /// # Errors
    ///
    /// - [`Error::NotFound`](crate::Error::NotFound) if either `from_id` or `to_id` does not refer to an
    ///   existing object.
    /// - [`Error::AlreadyExists`](crate::Error::AlreadyExists) if the association already exists.
    /// - [`Error::InvalidArgument`](crate::Error::InvalidArgument) if `label` or `properties` are malformed.
    async fn add(
        &self,
        from_id: Uuid,
        to_id: Uuid,
        label: &str,
        properties: Option<serde_json::Value>,
    ) -> Result<()>;

    /// Remove an association between two objects.
    ///
    /// The implementation should also remove the inverse association.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotFound`](crate::Error::NotFound) if no association with the given `label` exists
    /// between `from_id` and `to_id`.
    async fn remove(&self, from_id: Uuid, to_id: Uuid, label: &str) -> Result<()>;
}

// --- Blanket impls for Arc<T> ---

#[async_trait::async_trait]
impl<L: Label, T: ObjectStoreReader<L>> ObjectStoreReader<L> for Arc<T> {
    async fn get(&self, id: &Uuid) -> Result<Object<L>> {
        T::get(self, id).await
    }

    async fn get_by_name(&self, label: L, name: &ResourceName) -> Result<Object<L>> {
        T::get_by_name(self, label, name).await
    }

    async fn list(
        &self,
        label: L,
        namespace: Option<&ResourceName>,
        max_results: Option<usize>,
        page_token: Option<String>,
    ) -> Result<(Vec<Object<L>>, Option<String>)> {
        T::list(self, label, namespace, max_results, page_token).await
    }
}

#[async_trait::async_trait]
impl<L: Label, T: ObjectStore<L>> ObjectStore<L> for Arc<T> {
    async fn create(
        &self,
        label: L,
        name: &ResourceName,
        properties: Option<serde_json::Value>,
        id: Option<Uuid>,
    ) -> Result<Object<L>> {
        T::create(self, label, name, properties, id).await
    }

    async fn update(
        &self,
        id: &Uuid,
        properties: Option<serde_json::Value>,
        precondition: Precondition,
    ) -> Result<Object<L>> {
        T::update(self, id, properties, precondition).await
    }

    async fn rename(
        &self,
        id: &Uuid,
        new_name: &ResourceName,
        precondition: Precondition,
    ) -> Result<Object<L>> {
        T::rename(self, id, new_name, precondition).await
    }

    async fn delete(&self, id: &Uuid) -> Result<()> {
        T::delete(self, id).await
    }
}

#[async_trait::async_trait]
impl<L: Label, T: AssociationStoreReader<L>> AssociationStoreReader<L> for Arc<T> {
    async fn list(
        &self,
        from_id: Uuid,
        label: &str,
        target_label: Option<L>,
        max_results: Option<usize>,
        page_token: Option<String>,
    ) -> Result<(Vec<Association<L>>, Option<String>)> {
        T::list(self, from_id, label, target_label, max_results, page_token).await
    }
}

#[async_trait::async_trait]
impl<L: Label, T: AssociationStore<L>> AssociationStore<L> for Arc<T> {
    async fn add(
        &self,
        from_id: Uuid,
        to_id: Uuid,
        label: &str,
        properties: Option<serde_json::Value>,
    ) -> Result<()> {
        T::add(self, from_id, to_id, label, properties).await
    }

    async fn remove(&self, from_id: Uuid, to_id: Uuid, label: &str) -> Result<()> {
        T::remove(self, from_id, to_id, label).await
    }
}
