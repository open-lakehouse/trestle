use std::sync::Arc;

use bytes::Bytes;
use futures::future::BoxFuture;
use uuid::Uuid;

use crate::Result;
use crate::filter::Filter;
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

/// A JSON `null` reference, used to give payload-less rows a value to match against.
const NULL: serde_json::Value = serde_json::Value::Null;

/// The value a filter evaluates against for a possibly-absent payload: the stored
/// properties, or JSON `null` when there are none (so predicates see a missing path).
pub(crate) fn props_or_null(properties: &Option<serde_json::Value>) -> &serde_json::Value {
    properties.as_ref().unwrap_or(&NULL)
}

/// Parse a page token into an offset. Shared by the default `search` implementations, which
/// use the same plain-offset token shape as the backends' `list` methods.
pub(crate) fn parse_offset(page_token: Option<String>) -> Result<usize> {
    match page_token {
        Some(t) => t
            .parse()
            .map_err(|_| crate::Error::invalid_argument("invalid page token")),
        None => Ok(0),
    }
}

/// Apply `offset` and `max_results` over an already-filtered, already-ordered set and compute
/// the next token.
///
/// The default `search` impls filter the *entire* listing before paginating — never letting a
/// `LIMIT` truncate ahead of the filter — so paging a filtered result cannot drop matches.
/// A backend that pushes filtering into storage reuses this for the offset+limit slice.
pub(crate) fn paginate_filtered<T>(
    mut items: Vec<T>,
    offset: usize,
    max_results: Option<usize>,
) -> (Vec<T>, Option<String>) {
    let start = offset.min(items.len());
    items.drain(..start);
    let limit = max_results.unwrap_or(usize::MAX);
    let has_more = items.len() > limit;
    if has_more {
        items.truncate(limit);
    }
    let next = has_more.then(|| (offset + limit).to_string());
    (items, next)
}

/// The smallest possible UUIDv7 for a given instant: the millisecond timestamp in the high 48
/// bits and every other bit zero.
///
/// Edge ids are v7 (time-ordered), so this is a stable, index-friendly *time boundary* over the
/// `id` column: any real edge minted in millisecond `t` sorts at or after `v7_lower_bound(t)`
/// (its version nibble `0x7` and random tail only add to the value), and strictly before
/// `v7_lower_bound(t')` for any later millisecond `t'`. That makes a `[since, until)` window on
/// creation time expressible as `since_id <= id < until_id` without a separate `created_at`
/// index. Sub-millisecond precision in `t` is truncated to the millisecond (v7's resolution).
pub(crate) fn v7_lower_bound(t: chrono::DateTime<chrono::Utc>) -> Uuid {
    // Clamp to the representable 48-bit unix-millis range; negative (pre-epoch) maps to 0.
    let millis = t.timestamp_millis().max(0) as u128;
    // Layout: bits 127..80 = unix_ts_ms (48 bits), everything below zero.
    Uuid::from_u128(millis << 80)
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

    /// Search objects of a given label whose stored properties match `filter`,
    /// optionally scoped to a namespace prefix.
    ///
    /// Semantics mirror [`list`](Self::list) — same label scoping, same namespace prefix
    /// rule, same stable ordering, and the same offset-style pagination and token contract —
    /// with the sole addition of `filter`, applied to each object's stored
    /// [`properties`](Object::properties) using the reference semantics of
    /// [`Filter::matches`]. An object with no properties is matched against JSON `null`
    /// (predicates see a missing path). Filtering operates only on the plaintext payload;
    /// sensitive fields are sealed off the payload and are structurally unsearchable.
    ///
    /// The default implementation drains the full matching listing via [`list`](Self::list)
    /// and filters in process, so it is correct on any backend. Backends may override it to
    /// push the filter into storage, but must return results identical to the default (this
    /// is enforced by the shared conformance battery).
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidArgument`](crate::Error::InvalidArgument) if `page_token` is
    /// not a valid token for this query.
    async fn search(
        &self,
        label: L,
        namespace: Option<&ResourceName>,
        filter: &Filter,
        max_results: Option<usize>,
        page_token: Option<String>,
    ) -> Result<(Vec<Object<L>>, Option<String>)> {
        let offset = parse_offset(page_token)?;
        let mut matched = Vec::new();
        let mut token = None;
        loop {
            let (batch, next) = self.list(label, namespace, None, token).await?;
            matched.extend(
                batch
                    .into_iter()
                    .filter(|o| filter.matches(props_or_null(&o.properties))),
            );
            match next {
                Some(t) => token = Some(t),
                None => break,
            }
        }
        Ok(paginate_filtered(matched, offset, max_results))
    }

    /// Return the opaque sensitive blob stored alongside the object, if any.
    ///
    /// The blob is written by [`create`](ObjectStore::create) /
    /// [`update`](ObjectStore::update) and is kept off the public [`Object`] so it never
    /// serializes out through the ordinary read path — only this method exposes it. Backends
    /// that do not store a sensitive blob (the default) return `Ok(None)`.
    ///
    /// [`ManagedObjectStore`](crate::ManagedObjectStore) uses this to reconstitute sealed
    /// sensitive fields; the bytes are an `EnvelopeEncryptor` envelope, not plaintext (the
    /// `encryption` feature). It reads the object first, so a missing object surfaces there;
    /// callers of this method directly should treat `Ok(None)` as "no blob" regardless of
    /// whether the object exists.
    ///
    /// The default implementation returns `Ok(None)`, for backends that do not persist a
    /// sensitive blob.
    async fn get_sensitive(&self, id: &Uuid) -> Result<Option<Bytes>> {
        let _ = id;
        Ok(None)
    }
}

/// Read-write interface for the object store.
///
/// An object's [`label`](Object::label) — its resource *kind* — is fixed at
/// [`create`](ObjectStore::create) and is **immutable** thereafter: no method
/// takes a label to change it. Retyping an object is not an update or a rename;
/// it is conceptually a delete-and-recreate of a different kind, and a consumer
/// that genuinely needs it should model it as exactly that inside a
/// [`transaction`](Transactional::transaction) (delete the old object, create
/// the new one atomically).
#[async_trait::async_trait]
pub trait ObjectStore<L: Label>: ObjectStoreReader<L> + Send + Sync + 'static {
    /// Create a new object. The store generates `created_at` and `updated_at`.
    ///
    /// `id` lets the caller pre-allocate the object's id (e.g. a managed table
    /// adopting the id reserved by its staging reservation, or a managed volume
    /// embedding the id in its storage path); pass `None` to have the store
    /// generate a time-ordered id.
    ///
    /// `sensitive` is an opaque blob (typically an `EnvelopeEncryptor` envelope, from the
    /// `encryption` feature) persisted alongside the
    /// object and returned only by [`get_sensitive`](ObjectStoreReader::get_sensitive) — it
    /// never appears on the [`Object`] read back. It is stored atomically with the object, so
    /// there is no window in which one exists without the other. Pass `None` for objects with
    /// no sensitive data.
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
        sensitive: Option<Bytes>,
    ) -> Result<Object<L>>;

    /// Update an existing object's properties.
    ///
    /// The returned object carries the incremented [`version`](Object::version).
    /// Pass a [`Precondition::Version`] to make this a compare-and-swap and close
    /// the read-modify-write race; [`Precondition::Any`] overwrites
    /// unconditionally.
    ///
    /// `sensitive` replaces the object's stored sensitive blob when `Some`, atomically with
    /// the properties update. Passing `None` **leaves any existing blob untouched** (it does
    /// not clear it), so an update that does not carry sensitive fields preserves the sealed
    /// value already on the row.
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
        sensitive: Option<Bytes>,
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

    /// Replace **only** the object's opaque sensitive blob, leaving its properties,
    /// [`version`](Object::version), and `updated_at` untouched.
    ///
    /// This is the in-place counterpart to the `sensitive` parameter of
    /// [`create`](ObjectStore::create) / [`update`](ObjectStore::update): it is used to rewrite
    /// the sealed blob without touching the row's data — e.g. lazy KEK re-wrapping during a read,
    /// where bumping the version or overwriting properties would be wrong. Unlike `update`, it is
    /// **not** a versioned write and takes no [`Precondition`].
    ///
    /// The default implementation is a no-op returning `Ok(())`, for backends that do not persist
    /// a sensitive blob; backends that do must override it to rewrite the blob column only.
    ///
    /// # Errors
    ///
    /// Backends that persist a blob return [`Error::NotFound`](crate::Error::NotFound) if no
    /// object with `id` exists.
    async fn set_sensitive(&self, id: &Uuid, sensitive: Bytes) -> Result<()> {
        let _ = (id, sensitive);
        Ok(())
    }
}

/// The object an edge listing is anchored on, and thus the direction it walks.
///
/// TAO-style graphs traverse edges from a fixed endpoint: the *outgoing* edges of a
/// source object, or the *incoming* edges of a target object. [`From`](Self::From) walks
/// the former (matching the edge's `from_id`), [`Into`](Self::Into) the latter (matching
/// its `to_id`). Incoming listing is a direct reverse scan on `to_id`; it does **not** rely
/// on the inverse-edge mechanism (see [`AssociationStore`]) and so works regardless of
/// whether an inverse resolver is configured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeEndpoint {
    /// Outgoing edges from this source object — matches `from_id` (TAO `assoc_range`).
    From(Uuid),
    /// Incoming edges into this target object — matches `to_id` (a reverse scan).
    Into(Uuid),
}

/// A read query over edges, always ordered most-recent-first.
///
/// This is the single edge-read surface: it unifies direction ([`endpoint`](Self::endpoint)),
/// the restrictions that used to be positional arguments ([`target_label`](Self::target_label)
/// and an optional payload [`filter`](Self::filter)), a single-target restriction
/// ([`target_id`](Self::target_id), TAO `assoc_get`), and offset pagination.
///
/// Results are **always most-recent-first** — there is no ordering knob. The shipped backends
/// realize this by generating time-ordered (UUIDv7) edge ids and ordering by `id DESC`; a
/// caveat applies to edges created before v7 ids shipped (see [`query_edges`]).
///
/// Build one with [`EdgeQuery::from`] / [`EdgeQuery::into`] and the chaining setters:
///
/// ```
/// # use olai_store::{EdgeQuery, Filter};
/// # use uuid::Uuid;
/// # use olai_store::conformance::ConformanceLabel;
/// # let (src, dst) = (Uuid::nil(), Uuid::nil());
/// let filter = Filter::gt("weight", 4);
/// let q = EdgeQuery::<ConformanceLabel>::from(src, "link")
///     .target_id(dst)
///     .filter(&filter)
///     .page(Some(50), None);
/// # let _ = q;
/// ```
///
/// [`query_edges`]: AssociationStoreReader::query_edges
pub struct EdgeQuery<'a, L: Label> {
    /// Which object the listing is anchored on, and the direction it walks.
    pub endpoint: EdgeEndpoint,
    /// The edge label to list (required).
    pub label: &'a str,
    /// Restrict to edges whose target object (`to_label`) has this label.
    ///
    /// Only the target object's label is denormalized onto the edge row, so this filters
    /// `to_label` regardless of direction. For a [`From`](EdgeEndpoint::From) query that is the
    /// *other* endpoint (the useful case). For an [`Into`](EdgeEndpoint::Into) query the target
    /// **is** the anchor, so this can only match the anchor's own label — it cannot restrict the
    /// incoming edges by their *source* label (the source label is not stored; that would need a
    /// join). Leave it `None` on `Into` queries.
    pub target_label: Option<L>,
    /// Restrict to the single edge whose *other* endpoint is this object (TAO `assoc_get`).
    pub target_id: Option<Uuid>,
    /// Optional payload predicate over each edge's [`properties`](Association::properties).
    pub filter: Option<&'a Filter>,
    /// Inclusive lower bound on edge creation time — only edges created at or after this instant
    /// (TAO's "connections since T"). Resolved against the time-ordered edge id at millisecond
    /// precision.
    pub since: Option<chrono::DateTime<chrono::Utc>>,
    /// Exclusive upper bound on edge creation time — only edges created strictly before this
    /// instant. Resolved against the time-ordered edge id at millisecond precision.
    pub until: Option<chrono::DateTime<chrono::Utc>>,
    /// Maximum edges to return this page; `None` means no limit.
    pub max_results: Option<usize>,
    /// Continuation token from a previous page of the *same* query, or `None` to start.
    pub page_token: Option<String>,
}

impl<'a, L: Label> EdgeQuery<'a, L> {
    /// A query for the outgoing edges of `from_id` with edge `label`.
    pub fn from(from_id: Uuid, label: &'a str) -> Self {
        Self::anchored(EdgeEndpoint::From(from_id), label)
    }

    /// A query for the incoming edges into `to_id` with edge `label`.
    pub fn into(to_id: Uuid, label: &'a str) -> Self {
        Self::anchored(EdgeEndpoint::Into(to_id), label)
    }

    fn anchored(endpoint: EdgeEndpoint, label: &'a str) -> Self {
        Self {
            endpoint,
            label,
            target_label: None,
            target_id: None,
            filter: None,
            since: None,
            until: None,
            max_results: None,
            page_token: None,
        }
    }

    /// Restrict to edges whose target object (`to_label`) has this label.
    ///
    /// See [the field](Self::target_label): meaningful on [`from`](Self::from) queries; on
    /// [`into`](Self::into) queries it can only match the anchor's own label.
    #[must_use]
    pub fn target_label(mut self, target_label: L) -> Self {
        self.target_label = Some(target_label);
        self
    }

    /// Restrict to the single edge whose *other* endpoint is this object.
    ///
    /// The other endpoint is the target for a [`from`](Self::from) query and the source for an
    /// [`into`](Self::into) query, so this works correctly in both directions.
    #[must_use]
    pub fn target_id(mut self, target_id: Uuid) -> Self {
        self.target_id = Some(target_id);
        self
    }

    /// Apply a payload predicate.
    #[must_use]
    pub fn filter(mut self, filter: &'a Filter) -> Self {
        self.filter = Some(filter);
        self
    }

    /// Restrict to edges created at or after `since` (inclusive), TAO's "connections since T".
    ///
    /// Combines with [`until`](Self::until) to form the half-open window `[since, until)`.
    /// Resolved at millisecond precision against the time-ordered edge id.
    #[must_use]
    pub fn since(mut self, since: chrono::DateTime<chrono::Utc>) -> Self {
        self.since = Some(since);
        self
    }

    /// Restrict to edges created strictly before `until` (exclusive).
    ///
    /// Combines with [`since`](Self::since) to form the half-open window `[since, until)`.
    /// Resolved at millisecond precision against the time-ordered edge id.
    #[must_use]
    pub fn until(mut self, until: chrono::DateTime<chrono::Utc>) -> Self {
        self.until = Some(until);
        self
    }

    /// Set the page size and continuation token.
    #[must_use]
    pub fn page(mut self, max_results: Option<usize>, page_token: Option<String>) -> Self {
        self.max_results = max_results;
        self.page_token = page_token;
        self
    }
}

/// Read-only interface for the association (edge) store.
#[async_trait::async_trait]
pub trait AssociationStoreReader<L: Label>: Send + Sync + 'static {
    /// List edges matching an [`EdgeQuery`], most-recent-first.
    ///
    /// This is the one edge-read primitive: [`EdgeQuery`] carries the direction
    /// ([`endpoint`](EdgeQuery::endpoint)), the edge `label`, optional restriction to the other
    /// endpoint's object [`label`](EdgeQuery::target_label) or a specific
    /// [`id`](EdgeQuery::target_id), an optional payload [`filter`](EdgeQuery::filter), and
    /// offset pagination. At most `max_results` edges are returned per call; when more remain
    /// the returned token is `Some` and must be passed back as `page_token` to fetch the next
    /// page (a `None` token marks the final page). An edge with no properties is matched against
    /// JSON `null`.
    ///
    /// # Ordering
    ///
    /// Results are **most-recent-first**. The shipped backends realize this by generating
    /// time-ordered (UUIDv7) edge ids and ordering by `id` descending. Edges created before v7
    /// ids shipped carry random v4 ids, which sort before every v7 id, so any such legacy edge
    /// appears as "oldest" (last) and legacy edges have no meaningful order among themselves.
    /// No backfill is performed; edges written from now on are correctly time-ordered.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidArgument`](crate::Error::InvalidArgument) if `page_token` is not
    /// a valid token for this query.
    async fn query_edges(
        &self,
        query: EdgeQuery<'_, L>,
    ) -> Result<(Vec<Association<L>>, Option<String>)>;

    /// Count the edges matching an anchor + `label`, optionally restricted by target label
    /// (TAO `assoc_count`).
    ///
    /// The default implementation drains [`query_edges`](Self::query_edges) and counts; backends
    /// may override it with a `COUNT(*)` for efficiency but must return the same total.
    ///
    /// # Errors
    ///
    /// Returns a backend error if the count cannot be computed.
    async fn count_edges(
        &self,
        endpoint: EdgeEndpoint,
        label: &str,
        target_label: Option<L>,
    ) -> Result<u64> {
        let mut total: u64 = 0;
        let mut token = None;
        loop {
            let (batch, next) = self
                .query_edges(EdgeQuery {
                    endpoint,
                    label,
                    target_label,
                    target_id: None,
                    filter: None,
                    since: None,
                    until: None,
                    max_results: None,
                    page_token: token,
                })
                .await?;
            total += batch.len() as u64;
            match next {
                Some(t) => token = Some(t),
                None => break,
            }
        }
        Ok(total)
    }
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

    async fn search(
        &self,
        label: L,
        namespace: Option<&ResourceName>,
        filter: &Filter,
        max_results: Option<usize>,
        page_token: Option<String>,
    ) -> Result<(Vec<Object<L>>, Option<String>)> {
        T::search(self, label, namespace, filter, max_results, page_token).await
    }

    async fn get_sensitive(&self, id: &Uuid) -> Result<Option<Bytes>> {
        T::get_sensitive(self, id).await
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
        sensitive: Option<Bytes>,
    ) -> Result<Object<L>> {
        T::create(self, label, name, properties, id, sensitive).await
    }

    async fn update(
        &self,
        id: &Uuid,
        properties: Option<serde_json::Value>,
        precondition: Precondition,
        sensitive: Option<Bytes>,
    ) -> Result<Object<L>> {
        T::update(self, id, properties, precondition, sensitive).await
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

    async fn set_sensitive(&self, id: &Uuid, sensitive: Bytes) -> Result<()> {
        T::set_sensitive(self, id, sensitive).await
    }
}

#[async_trait::async_trait]
impl<L: Label, T: AssociationStoreReader<L>> AssociationStoreReader<L> for Arc<T> {
    async fn query_edges(
        &self,
        query: EdgeQuery<'_, L>,
    ) -> Result<(Vec<Association<L>>, Option<String>)> {
        T::query_edges(self, query).await
    }

    async fn count_edges(
        &self,
        endpoint: EdgeEndpoint,
        label: &str,
        target_label: Option<L>,
    ) -> Result<u64> {
        T::count_edges(self, endpoint, label, target_label).await
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

// --- Transactions ---

/// The object-safe execution surface shared by a top-level store and an open
/// transaction handle.
///
/// Every object and association operation is written once against
/// `&dyn StoreExec<L>`, so a repository function composes the same way whether it
/// runs standalone (auto-committing each call) or inside a
/// [`transaction`](Transactional::transaction). This mirrors the sqlx/sea-orm
/// idiom where the transaction handle implements the same trait as a plain
/// connection.
pub trait StoreExec<L: Label>: ObjectStore<L> + AssociationStore<L> {}

impl<L: Label, T: ObjectStore<L> + AssociationStore<L>> StoreExec<L> for T {}

/// An open transaction handle: a [`StoreExec`] whose writes are staged until
/// [`commit`](StoreTx::commit) (or discarded on [`rollback`](StoreTx::rollback)
/// / drop).
///
/// This is the escape hatch for imperative control flow that the closure form of
/// [`Transactional::transaction`] cannot express. Prefer the closure form, which
/// commits/rolls back automatically.
#[async_trait::async_trait]
pub trait StoreTx<L: Label>: StoreExec<L> {
    /// Commit the staged writes.
    ///
    /// # Errors
    ///
    /// Returns a backend error if the commit fails; the transaction is consumed
    /// regardless.
    async fn commit(self: Box<Self>) -> Result<()>;

    /// Discard the staged writes.
    ///
    /// Dropping the handle without committing has the same effect; this is the
    /// explicit form.
    ///
    /// # Errors
    ///
    /// Returns a backend error if the rollback fails.
    async fn rollback(self: Box<Self>) -> Result<()>;
}

/// A store that can run several operations as one atomic unit of work.
///
/// Two entry points, both backed by a real backend transaction:
///
/// - [`transaction`](Transactional::transaction) — the safe default. Runs a
///   closure against a borrowed [`StoreExec`]; commits on `Ok`, rolls back on
///   `Err`. Because of an async-closure lifetime limitation (the returned future
///   borrows the handle for a higher-ranked lifetime), the closure must return a
///   [`BoxFuture`] — write it as
///   `Box::new(|tx| Box::pin(async move { … }))`.
/// - [`begin`](Transactional::begin) — an explicit [`StoreTx`] handle for
///   imperative flows the closure form cannot express.
///
/// # Examples
///
/// ```no_run
/// # use olai_store::{Label, Result, Transactional, StoreExec};
/// # use futures::future::BoxFuture;
/// # async fn run<L: Label>(store: impl Transactional<L>) -> Result<()> {
/// let sum: i64 = store
///     .transaction(Box::new(|tx: &dyn StoreExec<L>| {
///         Box::pin(async move {
///             // ... tx.create(...).await?; tx.delete(...).await?; ...
///             Ok(42)
///         })
///     }))
///     .await?;
/// # let _ = sum;
/// # Ok(())
/// # }
/// ```
#[async_trait::async_trait]
pub trait Transactional<L: Label>: Send + Sync + 'static {
    /// Run a closure as a single atomic unit of work: commit on `Ok`, roll back
    /// on `Err`.
    ///
    /// # Errors
    ///
    /// Surfaces the closure's error (after rolling back), or a backend error
    /// from begin/commit.
    async fn transaction<'a, T>(
        &'a self,
        f: Box<dyn for<'t> FnOnce(&'t dyn StoreExec<L>) -> BoxFuture<'t, Result<T>> + Send + 'a>,
    ) -> Result<T>
    where
        T: Send + 'a;

    /// Begin an explicit transaction, returning a [`StoreTx`] handle.
    ///
    /// The caller is responsible for calling [`commit`](StoreTx::commit);
    /// dropping the handle rolls back.
    ///
    /// # Errors
    ///
    /// Returns a backend error if the transaction cannot be started.
    async fn begin(&self) -> Result<Box<dyn StoreTx<L>>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real v7 id sorts at or after its own millisecond's lower bound and strictly before the
    /// next millisecond's — the invariant the edge time-window range scan relies on.
    #[test]
    fn v7_lower_bound_brackets_real_ids() {
        let t = chrono::Utc::now();
        let lo = v7_lower_bound(t);
        let next = v7_lower_bound(t + chrono::Duration::milliseconds(1));

        // The boundary carries the timestamp with a zero tail, so it's <= any real v7 id in
        // that millisecond and < the next millisecond's boundary.
        assert!(lo < next, "consecutive millisecond bounds are ordered");
        let id = Uuid::new_v7(uuid::Timestamp::from_unix(
            uuid::NoContext,
            t.timestamp() as u64,
            t.timestamp_subsec_nanos(),
        ));
        assert!(id >= lo, "a real v7 id is at or after its ms lower bound");
        assert!(id < next, "and strictly before the next ms bound");
    }

    /// Pre-epoch instants clamp to the zero boundary rather than wrapping.
    #[test]
    fn v7_lower_bound_clamps_pre_epoch() {
        let pre = chrono::DateTime::<chrono::Utc>::from_timestamp(-100, 0).unwrap();
        assert_eq!(v7_lower_bound(pre), Uuid::from_u128(0));
    }
}
