//! In-memory reference backend.
//!
//! [`InMemoryStore`] is a small, dependency-free implementation of the store
//! traits — [`ObjectStore`], [`AssociationStore`], and [`Transactional`] — backed
//! by hash maps behind a lock. It is the zero-config default for `trestle new`
//! scaffolds, prototypes, and tests, and the reference the [conformance] battery
//! runs against. It is **not** intended for production: everything lives in
//! process memory and transactions serialize on a single lock.
//!
//! [conformance]: crate::conformance

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use uuid::Uuid;

use crate::label::Label;
use crate::name::ResourceName;
use crate::object::{Association, Object};
use crate::store::{
    AssociationStore, AssociationStoreReader, EdgeEndpoint, EdgeQuery, ObjectStore,
    ObjectStoreReader, Precondition, StoreExec, StoreTx, Transactional,
};
use crate::{Error, Result};

/// The OpenTelemetry `db.system` value for this backend's operation spans.
const DB_SYSTEM: &str = "memory";

/// Resolves an edge label to its paired inverse label, if any.
///
/// When an inverse label is returned, [`InMemoryStore`] maintains the inverse
/// edge alongside the primary one (see [`AssociationStore`]). The default
/// resolver returns `None` for every label (no inverse edges).
pub type InverseResolver = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// The committed state: objects keyed by id, plus a flat edge list.
///
/// The opaque `sensitive` blob (an envelope-encrypted payload written by
/// [`ManagedObjectStore`](crate::ManagedObjectStore)) is held in a sibling map rather than on
/// the [`Object`] so it stays out of the ordinary read path — only
/// [`get_sensitive`](ObjectStoreReader::get_sensitive) exposes it.
#[derive(Clone)]
struct State<L: Label> {
    objects: HashMap<Uuid, Object<L>>,
    sensitive: HashMap<Uuid, Bytes>,
    edges: Vec<Association<L>>,
}

impl<L: Label> Default for State<L> {
    fn default() -> Self {
        Self {
            objects: HashMap::new(),
            sensitive: HashMap::new(),
            edges: Vec::new(),
        }
    }
}

/// An in-memory [`ObjectStore`] + [`AssociationStore`] + [`Transactional`].
///
/// Cloning shares the same underlying state (it is `Arc`-backed), so clones are
/// cheap handles onto one store.
#[derive(Clone)]
pub struct InMemoryStore<L: Label> {
    state: Arc<Mutex<State<L>>>,
    /// Serializes transactions so a `transaction`/`begin` unit of work is
    /// isolated from concurrent writers. A `tokio` mutex so we can hold an owned
    /// `'static` guard across `.await` without unsafe lifetime tricks.
    txn_lock: Arc<tokio::sync::Mutex<()>>,
    inverse: InverseResolver,
}

impl<L: Label> Default for InMemoryStore<L> {
    fn default() -> Self {
        Self::new()
    }
}

impl<L: Label> InMemoryStore<L> {
    /// Create an empty store with no inverse-edge resolution.
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(State::default())),
            txn_lock: Arc::new(tokio::sync::Mutex::new(())),
            inverse: Arc::new(|_| None),
        }
    }

    /// Create an empty store that maintains inverse edges via `resolver`.
    ///
    /// Whenever an edge whose label the resolver maps to `Some(inverse)` is
    /// added or removed, the inverse edge (target → source, under `inverse`) is
    /// added or removed alongside it.
    pub fn with_inverse(resolver: impl Fn(&str) -> Option<String> + Send + Sync + 'static) -> Self {
        Self {
            state: Arc::new(Mutex::new(State::default())),
            txn_lock: Arc::new(tokio::sync::Mutex::new(())),
            inverse: Arc::new(resolver),
        }
    }
}

// --- Pure state operations, shared by the store and its transaction handle. ---
//
// Each takes `&mut State` so it can run against the live map (auto-commit) or a
// transaction's working snapshot identically.

fn op_create<L: Label>(
    state: &mut State<L>,
    label: L,
    name: &ResourceName,
    properties: Option<serde_json::Value>,
    id: Option<Uuid>,
    sensitive: Option<Bytes>,
) -> Result<Object<L>> {
    if state
        .objects
        .values()
        .any(|o| o.label == label && o.name.eq_ignore_ascii_case(name))
    {
        return Err(Error::AlreadyExists);
    }
    // UUIDv7 (time-ordered) so `id` doubles as the chronological keyset pagination key.
    let id = id.unwrap_or_else(Uuid::now_v7);
    if state.objects.contains_key(&id) {
        return Err(Error::AlreadyExists);
    }
    let object = Object {
        id,
        label,
        name: name.clone(),
        properties,
        version: 0,
        created_at: chrono::Utc::now(),
        updated_at: None,
    };
    state.objects.insert(id, object.clone());
    if let Some(blob) = sensitive {
        state.sensitive.insert(id, blob);
    }
    Ok(object)
}

fn op_update<L: Label>(
    state: &mut State<L>,
    id: &Uuid,
    properties: Option<serde_json::Value>,
    precondition: Precondition,
    sensitive: Option<Bytes>,
) -> Result<Object<L>> {
    let object = state.objects.get_mut(id).ok_or(Error::NotFound)?;
    precondition.check(object.version)?;
    object.properties = properties;
    object.version += 1;
    object.updated_at = Some(chrono::Utc::now());
    let updated = object.clone();
    // `None` preserves any existing blob; `Some` replaces it.
    if let Some(blob) = sensitive {
        state.sensitive.insert(*id, blob);
    }
    Ok(updated)
}

fn op_get_sensitive<L: Label>(state: &State<L>, id: &Uuid) -> Result<Option<Bytes>> {
    Ok(state.sensitive.get(id).cloned())
}

/// Replace only the sensitive blob, leaving the object (and its version) untouched.
fn op_set_sensitive<L: Label>(state: &mut State<L>, id: &Uuid, blob: Bytes) -> Result<()> {
    if !state.objects.contains_key(id) {
        return Err(Error::NotFound);
    }
    state.sensitive.insert(*id, blob);
    Ok(())
}

fn op_rename<L: Label>(
    state: &mut State<L>,
    id: &Uuid,
    new_name: &ResourceName,
    precondition: Precondition,
) -> Result<Object<L>> {
    let label = state.objects.get(id).ok_or(Error::NotFound)?.label;
    if state
        .objects
        .values()
        .any(|o| o.id != *id && o.label == label && o.name.eq_ignore_ascii_case(new_name))
    {
        return Err(Error::AlreadyExists);
    }
    let object = state.objects.get_mut(id).ok_or(Error::NotFound)?;
    precondition.check(object.version)?;
    object.name = new_name.clone();
    object.version += 1;
    object.updated_at = Some(chrono::Utc::now());
    Ok(object.clone())
}

fn op_delete<L: Label>(state: &mut State<L>, id: &Uuid) -> Result<()> {
    if state.objects.remove(id).is_none() {
        return Err(Error::NotFound);
    }
    // The sensitive blob rides the object; drop it in the same operation.
    state.sensitive.remove(id);
    // Cascade: drop every edge touching this object (either direction).
    state.edges.retain(|e| e.from_id != *id && e.to_id != *id);
    Ok(())
}

fn op_list_objects<L: Label>(
    state: &State<L>,
    label: L,
    namespace: Option<&ResourceName>,
    max_results: Option<usize>,
    page_token: Option<String>,
) -> Result<(Vec<Object<L>>, Option<String>)> {
    let q = crate::store::object_fingerprint(label, namespace, None);
    let cursor = crate::store::decode_cursor(page_token, q)?;
    let mut matches: Vec<Object<L>> = state
        .objects
        .values()
        .filter(|o| o.label == label && namespace.is_none_or(|ns| o.name.prefix_matches(ns)))
        .cloned()
        .collect();
    // Ascending id order (creation order for v7 ids); keyset resumes strictly past the cursor.
    matches.sort_by_key(|o| o.id);
    if let Some(k) = cursor {
        matches.retain(|o| o.id > k);
    }
    Ok(crate::store::paginate_keyset(
        matches,
        max_results,
        |o| o.id,
        q,
    ))
}

fn op_add_edge<L: Label>(
    state: &mut State<L>,
    from_id: Uuid,
    to_id: Uuid,
    label: &str,
    properties: Option<serde_json::Value>,
    inverse: &InverseResolver,
) -> Result<()> {
    let to_label = state.objects.get(&to_id).ok_or(Error::NotFound)?.label;
    if !state.objects.contains_key(&from_id) {
        return Err(Error::NotFound);
    }
    if state
        .edges
        .iter()
        .any(|e| e.from_id == from_id && e.to_id == to_id && e.label == label)
    {
        return Err(Error::AlreadyExists);
    }
    let from_label = state.objects.get(&from_id).map(|o| o.label);
    state.edges.push(Association {
        id: Uuid::now_v7(),
        from_id,
        label: label.to_string(),
        to_id,
        to_label,
        properties: properties.clone(),
        created_at: chrono::Utc::now(),
        updated_at: None,
    });
    // Maintain the inverse edge (target -> source) if the label has one.
    if let (Some(inv), Some(from_label)) = (inverse(label), from_label)
        && !state
            .edges
            .iter()
            .any(|e| e.from_id == to_id && e.to_id == from_id && e.label == inv)
    {
        state.edges.push(Association {
            id: Uuid::now_v7(),
            from_id: to_id,
            label: inv,
            to_id: from_id,
            to_label: from_label,
            properties,
            created_at: chrono::Utc::now(),
            updated_at: None,
        });
    }
    Ok(())
}

fn op_remove_edge<L: Label>(
    state: &mut State<L>,
    from_id: Uuid,
    to_id: Uuid,
    label: &str,
    inverse: &InverseResolver,
) -> Result<()> {
    let before = state.edges.len();
    state
        .edges
        .retain(|e| !(e.from_id == from_id && e.to_id == to_id && e.label == label));
    if state.edges.len() == before {
        return Err(Error::NotFound);
    }
    if let Some(inv) = inverse(label) {
        state
            .edges
            .retain(|e| !(e.from_id == to_id && e.to_id == from_id && e.label == inv));
    }
    Ok(())
}

fn op_query_edges<L: Label>(
    state: &State<L>,
    query: EdgeQuery<'_, L>,
) -> Result<(Vec<Association<L>>, Option<String>)> {
    let q = crate::store::edge_fingerprint(&query);
    let cursor = crate::store::decode_cursor(query.page_token.clone(), q)?;
    // The endpoint fixes which side we anchor on; `target_id` restricts the *other* side.
    let (into, anchor_id) = match query.endpoint {
        EdgeEndpoint::From(id) => (false, id),
        EdgeEndpoint::Into(id) => (true, id),
    };
    // For a `From` query the anchor is `from_id` and the other side is `to_id`; `Into` swaps them.
    let anchor = |e: &Association<L>| if into { e.to_id } else { e.from_id };
    let other = |e: &Association<L>| if into { e.from_id } else { e.to_id };
    // Time window as a v7 id range, matching the SQL backend: `[since_lo, until_lo)`.
    let since_lo = query.since.map(crate::store::v7_lower_bound);
    let until_lo = query.until.map(crate::store::v7_lower_bound);
    let mut matches: Vec<Association<L>> = state
        .edges
        .iter()
        .filter(|e| {
            anchor(e) == anchor_id
                && e.label == query.label
                && query.target_label.is_none_or(|tl| e.to_label == tl)
                && query.target_id.is_none_or(|tid| other(e) == tid)
                && since_lo.is_none_or(|lo| e.id >= lo)
                && until_lo.is_none_or(|lo| e.id < lo)
                && query
                    .filter
                    .is_none_or(|f| f.matches(crate::store::props_or_null(&e.properties)))
        })
        .cloned()
        .collect();
    // Most-recent-first: v7 edge ids are time-ordered, so descending id is descending time.
    matches.sort_by_key(|e| std::cmp::Reverse(e.id));
    // Keyset resumes strictly past the cursor; descending order means "after" is a smaller id.
    if let Some(k) = cursor {
        matches.retain(|e| e.id < k);
    }
    Ok(crate::store::paginate_keyset(
        matches,
        query.max_results,
        |e| e.id,
        q,
    ))
}

// --- Top-level store: each op takes the state lock and auto-commits. ---

/// Record the failure of `result` onto the current span's OpenTelemetry status fields.
///
/// The span must declare `otel.status_code` and `error.type` as
/// [`tracing::field::Empty`] for these to take effect. Only the error *kind*
/// ([`Error::kind_str`]) is recorded — never a payload or message body.
fn record_err<T>(result: &Result<T>) {
    if let Err(e) = result {
        let span = tracing::Span::current();
        span.record("otel.status_code", "ERROR");
        span.record("error.type", e.kind_str());
    }
}

#[async_trait::async_trait]
impl<L: Label> ObjectStoreReader<L> for InMemoryStore<L> {
    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.get",
            otel.kind = "client",
            db.system = DB_SYSTEM,
            db.operation.name = "get",
            id = %id,
            otel.status_code = tracing::field::Empty,
            error.type = tracing::field::Empty,
        )
    )]
    async fn get(&self, id: &Uuid) -> Result<Object<L>> {
        let out = self
            .state
            .lock()
            .unwrap()
            .objects
            .get(id)
            .cloned()
            .ok_or(Error::NotFound);
        record_err(&out);
        out
    }

    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.get_by_name",
            otel.kind = "client",
            db.system = DB_SYSTEM,
            db.operation.name = "get_by_name",
            db.collection.name = label.as_str(),
            name = %name,
            otel.status_code = tracing::field::Empty,
            error.type = tracing::field::Empty,
        )
    )]
    async fn get_by_name(&self, label: L, name: &ResourceName) -> Result<Object<L>> {
        let out = self
            .state
            .lock()
            .unwrap()
            .objects
            .values()
            .find(|o| o.label == label && o.name.eq_ignore_ascii_case(name))
            .cloned()
            .ok_or(Error::NotFound);
        record_err(&out);
        out
    }

    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.list",
            otel.kind = "client",
            db.system = DB_SYSTEM,
            db.operation.name = "list",
            db.collection.name = label.as_str(),
            max_results = ?max_results,
            otel.status_code = tracing::field::Empty,
            error.type = tracing::field::Empty,
        )
    )]
    async fn list(
        &self,
        label: L,
        namespace: Option<&ResourceName>,
        max_results: Option<usize>,
        page_token: Option<String>,
    ) -> Result<(Vec<Object<L>>, Option<String>)> {
        let out = op_list_objects(
            &self.state.lock().unwrap(),
            label,
            namespace,
            max_results,
            page_token,
        );
        record_err(&out);
        out
    }

    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.get_sensitive",
            otel.kind = "client",
            db.system = DB_SYSTEM,
            db.operation.name = "get_sensitive",
            id = %id,
            otel.status_code = tracing::field::Empty,
            error.type = tracing::field::Empty,
        )
    )]
    async fn get_sensitive(&self, id: &Uuid) -> Result<Option<Bytes>> {
        let out = op_get_sensitive(&self.state.lock().unwrap(), id);
        record_err(&out);
        out
    }
}

/// Emit a `debug` event when `result` is a CAS (`Precondition::Version`) conflict.
///
/// Records only the object `id` — the `Conflict` variant carries no payload.
fn debug_cas_conflict<T>(result: &Result<T>, id: &Uuid) {
    if let Err(Error::Conflict) = result {
        tracing::debug!(id = %id, "CAS precondition conflict");
    }
}

#[async_trait::async_trait]
impl<L: Label> ObjectStore<L> for InMemoryStore<L> {
    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.create",
            otel.kind = "client",
            db.system = DB_SYSTEM,
            db.operation.name = "create",
            db.collection.name = label.as_str(),
            name = %name,
            otel.status_code = tracing::field::Empty,
            error.type = tracing::field::Empty,
        )
    )]
    async fn create(
        &self,
        label: L,
        name: &ResourceName,
        properties: Option<serde_json::Value>,
        id: Option<Uuid>,
        sensitive: Option<Bytes>,
    ) -> Result<Object<L>> {
        let out = op_create(
            &mut self.state.lock().unwrap(),
            label,
            name,
            properties,
            id,
            sensitive,
        );
        record_err(&out);
        out
    }

    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.update",
            otel.kind = "client",
            db.system = DB_SYSTEM,
            db.operation.name = "update",
            id = %id,
            precondition = ?precondition,
            otel.status_code = tracing::field::Empty,
            error.type = tracing::field::Empty,
        )
    )]
    async fn update(
        &self,
        id: &Uuid,
        properties: Option<serde_json::Value>,
        precondition: Precondition,
        sensitive: Option<Bytes>,
    ) -> Result<Object<L>> {
        let out = op_update(
            &mut self.state.lock().unwrap(),
            id,
            properties,
            precondition,
            sensitive,
        );
        record_err(&out);
        debug_cas_conflict(&out, id);
        out
    }

    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.rename",
            otel.kind = "client",
            db.system = DB_SYSTEM,
            db.operation.name = "rename",
            id = %id,
            name = %new_name,
            precondition = ?precondition,
            otel.status_code = tracing::field::Empty,
            error.type = tracing::field::Empty,
        )
    )]
    async fn rename(
        &self,
        id: &Uuid,
        new_name: &ResourceName,
        precondition: Precondition,
    ) -> Result<Object<L>> {
        let out = op_rename(&mut self.state.lock().unwrap(), id, new_name, precondition);
        record_err(&out);
        debug_cas_conflict(&out, id);
        out
    }

    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.delete",
            otel.kind = "client",
            db.system = DB_SYSTEM,
            db.operation.name = "delete",
            id = %id,
            otel.status_code = tracing::field::Empty,
            error.type = tracing::field::Empty,
        )
    )]
    async fn delete(&self, id: &Uuid) -> Result<()> {
        let out = op_delete(&mut self.state.lock().unwrap(), id);
        record_err(&out);
        out
    }

    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.set_sensitive",
            otel.kind = "client",
            db.system = DB_SYSTEM,
            db.operation.name = "set_sensitive",
            id = %id,
            otel.status_code = tracing::field::Empty,
            error.type = tracing::field::Empty,
        )
    )]
    async fn set_sensitive(&self, id: &Uuid, sensitive: Bytes) -> Result<()> {
        let out = op_set_sensitive(&mut self.state.lock().unwrap(), id, sensitive);
        record_err(&out);
        out
    }
}

#[async_trait::async_trait]
impl<L: Label> AssociationStoreReader<L> for InMemoryStore<L> {
    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.query_edges",
            otel.kind = "client",
            db.system = DB_SYSTEM,
            db.operation.name = "query_edges",
            label = %query.label,
            otel.status_code = tracing::field::Empty,
            error.type = tracing::field::Empty,
        )
    )]
    async fn query_edges(
        &self,
        query: EdgeQuery<'_, L>,
    ) -> Result<(Vec<Association<L>>, Option<String>)> {
        let out = op_query_edges(&self.state.lock().unwrap(), query);
        record_err(&out);
        out
    }
}

#[async_trait::async_trait]
impl<L: Label> AssociationStore<L> for InMemoryStore<L> {
    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.add_edge",
            otel.kind = "client",
            db.system = DB_SYSTEM,
            db.operation.name = "add_edge",
            label = %label,
            from_id = %from_id,
            to_id = %to_id,
            otel.status_code = tracing::field::Empty,
            error.type = tracing::field::Empty,
        )
    )]
    async fn add(
        &self,
        from_id: Uuid,
        to_id: Uuid,
        label: &str,
        properties: Option<serde_json::Value>,
    ) -> Result<()> {
        let out = op_add_edge(
            &mut self.state.lock().unwrap(),
            from_id,
            to_id,
            label,
            properties,
            &self.inverse,
        );
        record_err(&out);
        out
    }

    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.remove_edge",
            otel.kind = "client",
            db.system = DB_SYSTEM,
            db.operation.name = "remove_edge",
            label = %label,
            from_id = %from_id,
            to_id = %to_id,
            otel.status_code = tracing::field::Empty,
            error.type = tracing::field::Empty,
        )
    )]
    async fn remove(&self, from_id: Uuid, to_id: Uuid, label: &str) -> Result<()> {
        let out = op_remove_edge(
            &mut self.state.lock().unwrap(),
            from_id,
            to_id,
            label,
            &self.inverse,
        );
        record_err(&out);
        out
    }
}

// --- Transactions: snapshot the state, run ops against the snapshot, swap on
//     commit. A dedicated lock serializes units of work. ---

/// An open in-memory transaction: buffers writes in a cloned snapshot until
/// [`commit`](StoreTx::commit).
pub struct InMemoryTx<L: Label> {
    store: InMemoryStore<L>,
    /// The working snapshot; `None` once committed/rolled back.
    working: Mutex<Option<State<L>>>,
    /// Held for the lifetime of the transaction to serialize units of work.
    /// An owned `'static` guard — no unsafe lifetime extension needed.
    _guard: tokio::sync::OwnedMutexGuard<()>,
}

impl<L: Label> InMemoryStore<L> {
    /// Take the txn lock and snapshot current state into a new transaction.
    async fn begin_tx(&self) -> InMemoryTx<L> {
        let guard = self.txn_lock.clone().lock_owned().await;
        let snapshot = self.state.lock().unwrap().clone();
        InMemoryTx {
            store: self.clone(),
            working: Mutex::new(Some(snapshot)),
            _guard: guard,
        }
    }
}

impl<L: Label> InMemoryTx<L> {
    fn with_state<T>(&self, f: impl FnOnce(&mut State<L>) -> Result<T>) -> Result<T> {
        let mut guard = self.working.lock().unwrap();
        let state = guard
            .as_mut()
            .ok_or_else(|| Error::generic("transaction already committed or rolled back"))?;
        f(state)
    }
}

#[async_trait::async_trait]
impl<L: Label> ObjectStoreReader<L> for InMemoryTx<L> {
    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.get",
            otel.kind = "client",
            db.system = DB_SYSTEM,
            db.operation.name = "get",
            id = %id,
            otel.status_code = tracing::field::Empty,
            error.type = tracing::field::Empty,
        )
    )]
    async fn get(&self, id: &Uuid) -> Result<Object<L>> {
        let out = self.with_state(|s| s.objects.get(id).cloned().ok_or(Error::NotFound));
        record_err(&out);
        out
    }

    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.get_by_name",
            otel.kind = "client",
            db.system = DB_SYSTEM,
            db.operation.name = "get_by_name",
            db.collection.name = label.as_str(),
            name = %name,
            otel.status_code = tracing::field::Empty,
            error.type = tracing::field::Empty,
        )
    )]
    async fn get_by_name(&self, label: L, name: &ResourceName) -> Result<Object<L>> {
        let out = self.with_state(|s| {
            s.objects
                .values()
                .find(|o| o.label == label && o.name.eq_ignore_ascii_case(name))
                .cloned()
                .ok_or(Error::NotFound)
        });
        record_err(&out);
        out
    }

    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.list",
            otel.kind = "client",
            db.system = DB_SYSTEM,
            db.operation.name = "list",
            db.collection.name = label.as_str(),
            max_results = ?max_results,
            otel.status_code = tracing::field::Empty,
            error.type = tracing::field::Empty,
        )
    )]
    async fn list(
        &self,
        label: L,
        namespace: Option<&ResourceName>,
        max_results: Option<usize>,
        page_token: Option<String>,
    ) -> Result<(Vec<Object<L>>, Option<String>)> {
        let out =
            self.with_state(|s| op_list_objects(s, label, namespace, max_results, page_token));
        record_err(&out);
        out
    }

    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.get_sensitive",
            otel.kind = "client",
            db.system = DB_SYSTEM,
            db.operation.name = "get_sensitive",
            id = %id,
            otel.status_code = tracing::field::Empty,
            error.type = tracing::field::Empty,
        )
    )]
    async fn get_sensitive(&self, id: &Uuid) -> Result<Option<Bytes>> {
        let out = self.with_state(|s| op_get_sensitive(s, id));
        record_err(&out);
        out
    }
}

#[async_trait::async_trait]
impl<L: Label> ObjectStore<L> for InMemoryTx<L> {
    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.create",
            otel.kind = "client",
            db.system = DB_SYSTEM,
            db.operation.name = "create",
            db.collection.name = label.as_str(),
            name = %name,
            otel.status_code = tracing::field::Empty,
            error.type = tracing::field::Empty,
        )
    )]
    async fn create(
        &self,
        label: L,
        name: &ResourceName,
        properties: Option<serde_json::Value>,
        id: Option<Uuid>,
        sensitive: Option<Bytes>,
    ) -> Result<Object<L>> {
        let out = self.with_state(|s| op_create(s, label, name, properties, id, sensitive));
        record_err(&out);
        out
    }

    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.update",
            otel.kind = "client",
            db.system = DB_SYSTEM,
            db.operation.name = "update",
            id = %id,
            precondition = ?precondition,
            otel.status_code = tracing::field::Empty,
            error.type = tracing::field::Empty,
        )
    )]
    async fn update(
        &self,
        id: &Uuid,
        properties: Option<serde_json::Value>,
        precondition: Precondition,
        sensitive: Option<Bytes>,
    ) -> Result<Object<L>> {
        let out = self.with_state(|s| op_update(s, id, properties, precondition, sensitive));
        record_err(&out);
        debug_cas_conflict(&out, id);
        out
    }

    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.rename",
            otel.kind = "client",
            db.system = DB_SYSTEM,
            db.operation.name = "rename",
            id = %id,
            name = %new_name,
            precondition = ?precondition,
            otel.status_code = tracing::field::Empty,
            error.type = tracing::field::Empty,
        )
    )]
    async fn rename(
        &self,
        id: &Uuid,
        new_name: &ResourceName,
        precondition: Precondition,
    ) -> Result<Object<L>> {
        let out = self.with_state(|s| op_rename(s, id, new_name, precondition));
        record_err(&out);
        debug_cas_conflict(&out, id);
        out
    }

    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.delete",
            otel.kind = "client",
            db.system = DB_SYSTEM,
            db.operation.name = "delete",
            id = %id,
            otel.status_code = tracing::field::Empty,
            error.type = tracing::field::Empty,
        )
    )]
    async fn delete(&self, id: &Uuid) -> Result<()> {
        let out = self.with_state(|s| op_delete(s, id));
        record_err(&out);
        out
    }

    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.set_sensitive",
            otel.kind = "client",
            db.system = DB_SYSTEM,
            db.operation.name = "set_sensitive",
            id = %id,
            otel.status_code = tracing::field::Empty,
            error.type = tracing::field::Empty,
        )
    )]
    async fn set_sensitive(&self, id: &Uuid, sensitive: Bytes) -> Result<()> {
        let out = self.with_state(|s| op_set_sensitive(s, id, sensitive));
        record_err(&out);
        out
    }
}

#[async_trait::async_trait]
impl<L: Label> AssociationStoreReader<L> for InMemoryTx<L> {
    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.query_edges",
            otel.kind = "client",
            db.system = DB_SYSTEM,
            db.operation.name = "query_edges",
            label = %query.label,
            otel.status_code = tracing::field::Empty,
            error.type = tracing::field::Empty,
        )
    )]
    async fn query_edges(
        &self,
        query: EdgeQuery<'_, L>,
    ) -> Result<(Vec<Association<L>>, Option<String>)> {
        let out = self.with_state(|s| op_query_edges(s, query));
        record_err(&out);
        out
    }
}

#[async_trait::async_trait]
impl<L: Label> AssociationStore<L> for InMemoryTx<L> {
    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.add_edge",
            otel.kind = "client",
            db.system = DB_SYSTEM,
            db.operation.name = "add_edge",
            label = %label,
            from_id = %from_id,
            to_id = %to_id,
            otel.status_code = tracing::field::Empty,
            error.type = tracing::field::Empty,
        )
    )]
    async fn add(
        &self,
        from_id: Uuid,
        to_id: Uuid,
        label: &str,
        properties: Option<serde_json::Value>,
    ) -> Result<()> {
        let inverse = self.store.inverse.clone();
        let out = self.with_state(|s| op_add_edge(s, from_id, to_id, label, properties, &inverse));
        record_err(&out);
        out
    }

    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.remove_edge",
            otel.kind = "client",
            db.system = DB_SYSTEM,
            db.operation.name = "remove_edge",
            label = %label,
            from_id = %from_id,
            to_id = %to_id,
            otel.status_code = tracing::field::Empty,
            error.type = tracing::field::Empty,
        )
    )]
    async fn remove(&self, from_id: Uuid, to_id: Uuid, label: &str) -> Result<()> {
        let inverse = self.store.inverse.clone();
        let out = self.with_state(|s| op_remove_edge(s, from_id, to_id, label, &inverse));
        record_err(&out);
        out
    }
}

#[async_trait::async_trait]
impl<L: Label> StoreTx<L> for InMemoryTx<L> {
    #[tracing::instrument(
        skip_all,
        fields(otel.kind = "client", db.system = DB_SYSTEM, db.operation.name = "commit")
    )]
    async fn commit(self: Box<Self>) -> Result<()> {
        if let Some(working) = self.working.lock().unwrap().take() {
            *self.store.state.lock().unwrap() = working;
        }
        Ok(())
    }

    #[tracing::instrument(
        skip_all,
        fields(otel.kind = "client", db.system = DB_SYSTEM, db.operation.name = "rollback")
    )]
    async fn rollback(self: Box<Self>) -> Result<()> {
        *self.working.lock().unwrap() = None;
        Ok(())
    }
}

#[async_trait::async_trait]
impl<L: Label> Transactional<L> for InMemoryStore<L> {
    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.transaction",
            otel.kind = "client",
            db.system = DB_SYSTEM,
            db.operation.name = "transaction",
            otel.status_code = tracing::field::Empty,
            error.type = tracing::field::Empty,
        )
    )]
    async fn transaction<'a, T>(
        &'a self,
        f: Box<
            dyn for<'t> FnOnce(&'t dyn StoreExec<L>) -> futures::future::BoxFuture<'t, Result<T>>
                + Send
                + 'a,
        >,
    ) -> Result<T>
    where
        T: Send + 'a,
    {
        let tx = self.begin_tx().await;
        let outcome = f(&tx).await;
        match outcome {
            Ok(value) => {
                if let Some(working) = tx.working.lock().unwrap().take() {
                    *self.state.lock().unwrap() = working;
                }
                Ok(value)
            }
            Err(e) => {
                // rollback: discard the working snapshot
                let span = tracing::Span::current();
                span.record("otel.status_code", "ERROR");
                span.record("error.type", e.kind_str());
                *tx.working.lock().unwrap() = None;
                Err(e)
            }
        }
    }

    #[tracing::instrument(
        skip_all,
        fields(otel.kind = "client", db.system = DB_SYSTEM, db.operation.name = "begin")
    )]
    async fn begin(&self) -> Result<Box<dyn StoreTx<L>>> {
        Ok(Box::new(self.begin_tx().await))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt;
    use std::str::FromStr;

    #[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
    enum Kind {
        Node,
    }
    impl fmt::Display for Kind {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("node")
        }
    }
    impl FromStr for Kind {
        type Err = String;
        fn from_str(s: &str) -> std::result::Result<Self, String> {
            match s {
                "node" => Ok(Kind::Node),
                other => Err(format!("unknown: {other}")),
            }
        }
    }
    impl Label for Kind {
        fn as_str(&self) -> &str {
            "node"
        }
    }

    fn rn(s: &str) -> ResourceName {
        ResourceName::from_naive_str_split(s)
    }

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn cas_update_detects_conflict() {
        let store = InMemoryStore::<Kind>::new();
        let obj = store
            .create(Kind::Node, &rn("a"), None, None, None)
            .await
            .unwrap();
        assert_eq!(obj.version, 0);

        // Fresh version succeeds and bumps.
        let updated = store
            .update(&obj.id, None, Precondition::Version(0), None)
            .await
            .unwrap();
        assert_eq!(updated.version, 1);

        // Stale version conflicts.
        let err = store
            .update(&obj.id, None, Precondition::Version(0), None)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Conflict));

        // The conflict is surfaced as a `debug` event for operators.
        assert!(logs_contain("CAS precondition conflict"));

        // `Any` still works unconditionally.
        let again = store
            .update(&obj.id, None, Precondition::Any, None)
            .await
            .unwrap();
        assert_eq!(again.version, 2);
    }

    #[tokio::test]
    async fn rename_preserves_id_and_associations_and_rejects_collision() {
        let store = InMemoryStore::<Kind>::new();
        let a = store
            .create(Kind::Node, &rn("a"), None, None, None)
            .await
            .unwrap();
        let b = store
            .create(Kind::Node, &rn("b"), None, None, None)
            .await
            .unwrap();
        store.add(a.id, b.id, "link", None).await.unwrap();

        // Renaming onto an existing name collides.
        let err = store
            .rename(&a.id, &rn("b"), Precondition::Any)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::AlreadyExists));

        // A fresh name preserves id + edges.
        let renamed = store
            .rename(&a.id, &rn("a2"), Precondition::Version(a.version))
            .await
            .unwrap();
        assert_eq!(renamed.id, a.id);
        assert_eq!(renamed.name, rn("a2"));
        let (edges, _) = store
            .query_edges(EdgeQuery::from(a.id, "link"))
            .await
            .unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to_id, b.id);
    }

    #[tokio::test]
    async fn transaction_rolls_back_on_err() {
        let store = InMemoryStore::<Kind>::new();
        let seed = store
            .create(Kind::Node, &rn("seed"), None, None, None)
            .await
            .unwrap();

        let store2 = store.clone();
        let seed_id = seed.id;
        let res: Result<()> = store
            .transaction(Box::new(move |tx| {
                Box::pin(async move {
                    tx.delete(&seed_id).await?;
                    tx.create(Kind::Node, &rn("new"), None, None, None).await?;
                    Err(Error::generic("boom"))
                })
            }))
            .await;
        assert!(res.is_err());

        // Rolled back: seed still present, "new" never created.
        assert!(store2.get(&seed_id).await.is_ok());
        assert!(store2.get_by_name(Kind::Node, &rn("new")).await.is_err());
    }

    #[tokio::test]
    async fn transaction_commits_on_ok() {
        let store = InMemoryStore::<Kind>::new();
        let res: Result<Uuid> = store
            .transaction(Box::new(|tx| {
                Box::pin(async move {
                    let a = tx.create(Kind::Node, &rn("x"), None, None, None).await?;
                    let b = tx.create(Kind::Node, &rn("y"), None, None, None).await?;
                    tx.add(a.id, b.id, "e", None).await?;
                    Ok(a.id)
                })
            }))
            .await;
        let a_id = res.unwrap();
        assert!(store.get(&a_id).await.is_ok());
        let (edges, _) = store.query_edges(EdgeQuery::from(a_id, "e")).await.unwrap();
        assert_eq!(edges.len(), 1);
    }

    #[tokio::test]
    async fn inverse_edges_maintained() {
        let store = InMemoryStore::<Kind>::with_inverse(|l| match l {
            "parent_of" => Some("child_of".to_string()),
            _ => None,
        });
        let p = store
            .create(Kind::Node, &rn("p"), None, None, None)
            .await
            .unwrap();
        let c = store
            .create(Kind::Node, &rn("c"), None, None, None)
            .await
            .unwrap();

        store.add(p.id, c.id, "parent_of", None).await.unwrap();
        // Forward edge present.
        let (fwd, _) = store
            .query_edges(EdgeQuery::from(p.id, "parent_of"))
            .await
            .unwrap();
        assert_eq!(fwd.len(), 1);
        // Inverse edge present.
        let (inv, _) = store
            .query_edges(EdgeQuery::from(c.id, "child_of"))
            .await
            .unwrap();
        assert_eq!(inv.len(), 1);
        assert_eq!(inv[0].to_id, p.id);

        // Removing the forward edge removes the inverse too.
        store.remove(p.id, c.id, "parent_of").await.unwrap();
        let (inv, _) = store
            .query_edges(EdgeQuery::from(c.id, "child_of"))
            .await
            .unwrap();
        assert!(inv.is_empty());
    }

    /// The `skip_all` guardrail: no operation span/event may echo the properties payload or a
    /// sensitive blob. Drive a write whose payload and sealed blob carry sentinels and assert
    /// neither ever appears in emitted tracing output.
    #[tokio::test]
    #[tracing_test::traced_test]
    async fn spans_never_leak_payload_or_secret() {
        let store = InMemoryStore::<Kind>::new();
        let created = store
            .create(
                Kind::Node,
                &rn("n"),
                Some(serde_json::json!({ "field": "SECRET_SENTINEL_VALUE" })),
                None,
                Some(Bytes::from_static(b"SECRET_BLOB_BYTES")),
            )
            .await
            .unwrap();
        // Exercise more instrumented paths carrying the same payload.
        store
            .update(
                &created.id,
                Some(serde_json::json!({ "field": "SECRET_SENTINEL_VALUE" })),
                Precondition::Any,
                Some(Bytes::from_static(b"SECRET_BLOB_BYTES")),
            )
            .await
            .unwrap();
        let _ = store.get(&created.id).await.unwrap();

        logs_assert(|lines: &[&str]| {
            for line in lines {
                if line.contains("SECRET_SENTINEL_VALUE") {
                    return Err(format!(
                        "properties payload leaked into tracing output: {line}"
                    ));
                }
                if line.contains("SECRET_BLOB_BYTES") {
                    return Err(format!("sensitive blob leaked into tracing output: {line}"));
                }
            }
            Ok(())
        });
    }

    #[tokio::test]
    async fn list_paginates_deterministically() {
        let store = InMemoryStore::<Kind>::new();
        for i in 0..5 {
            store
                .create(Kind::Node, &rn(&format!("n{i}")), None, None, None)
                .await
                .unwrap();
        }
        let (first, tok) = ObjectStoreReader::list(&store, Kind::Node, None, Some(2), None)
            .await
            .unwrap();
        assert_eq!(first.len(), 2);
        let tok = tok.expect("more pages");
        let (second, tok2) = ObjectStoreReader::list(&store, Kind::Node, None, Some(2), Some(tok))
            .await
            .unwrap();
        assert_eq!(second.len(), 2);
        assert!(tok2.is_some());
        // No overlap between pages.
        assert_ne!(first[0].id, second[0].id);
    }
}
