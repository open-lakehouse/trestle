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

use uuid::Uuid;

use crate::label::Label;
use crate::name::ResourceName;
use crate::object::{Association, Object};
use crate::store::{
    AssociationStore, AssociationStoreReader, ObjectStore, ObjectStoreReader, Precondition,
    StoreExec, StoreTx, Transactional,
};
use crate::{Error, Result};

/// Resolves an edge label to its paired inverse label, if any.
///
/// When an inverse label is returned, [`InMemoryStore`] maintains the inverse
/// edge alongside the primary one (see [`AssociationStore`]). The default
/// resolver returns `None` for every label (no inverse edges).
pub type InverseResolver = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// The committed state: objects keyed by id, plus a flat edge list.
#[derive(Clone)]
struct State<L: Label> {
    objects: HashMap<Uuid, Object<L>>,
    edges: Vec<Association<L>>,
}

impl<L: Label> Default for State<L> {
    fn default() -> Self {
        Self {
            objects: HashMap::new(),
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
) -> Result<Object<L>> {
    if state
        .objects
        .values()
        .any(|o| o.label == label && &o.name == name)
    {
        return Err(Error::AlreadyExists);
    }
    let id = id.unwrap_or_else(Uuid::new_v4);
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
    Ok(object)
}

fn op_update<L: Label>(
    state: &mut State<L>,
    id: &Uuid,
    properties: Option<serde_json::Value>,
    precondition: Precondition,
) -> Result<Object<L>> {
    let object = state.objects.get_mut(id).ok_or(Error::NotFound)?;
    precondition.check(object.version)?;
    object.properties = properties;
    object.version += 1;
    object.updated_at = Some(chrono::Utc::now());
    Ok(object.clone())
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
        .any(|o| o.id != *id && o.label == label && &o.name == new_name)
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
    let mut matches: Vec<Object<L>> = state
        .objects
        .values()
        .filter(|o| o.label == label && namespace.is_none_or(|ns| o.name.prefix_matches(ns)))
        .cloned()
        .collect();
    // Stable order for deterministic paging: by id.
    matches.sort_by_key(|o| o.id);
    paginate(matches, max_results, page_token)
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
        id: Uuid::new_v4(),
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
            id: Uuid::new_v4(),
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

fn op_list_edges<L: Label>(
    state: &State<L>,
    from_id: Uuid,
    label: &str,
    target_label: Option<L>,
    max_results: Option<usize>,
    page_token: Option<String>,
) -> Result<(Vec<Association<L>>, Option<String>)> {
    let mut matches: Vec<Association<L>> = state
        .edges
        .iter()
        .filter(|e| {
            e.from_id == from_id
                && e.label == label
                && target_label.is_none_or(|tl| e.to_label == tl)
        })
        .cloned()
        .collect();
    matches.sort_by_key(|e| e.id);
    paginate(matches, max_results, page_token)
}

/// Offset-based pagination over an already-ordered vec. The token is the
/// stringified next offset.
fn paginate<T>(
    items: Vec<T>,
    max_results: Option<usize>,
    page_token: Option<String>,
) -> Result<(Vec<T>, Option<String>)> {
    let start: usize = match page_token {
        Some(tok) => tok
            .parse()
            .map_err(|_| Error::invalid_argument("invalid page token"))?,
        None => 0,
    };
    let limit = max_results.unwrap_or(usize::MAX);
    let end = start.saturating_add(limit).min(items.len());
    let start = start.min(items.len());
    let next = if end < items.len() {
        Some(end.to_string())
    } else {
        None
    };
    Ok((
        items.into_iter().skip(start).take(end - start).collect(),
        next,
    ))
}

// --- Top-level store: each op takes the state lock and auto-commits. ---

#[async_trait::async_trait]
impl<L: Label> ObjectStoreReader<L> for InMemoryStore<L> {
    async fn get(&self, id: &Uuid) -> Result<Object<L>> {
        self.state
            .lock()
            .unwrap()
            .objects
            .get(id)
            .cloned()
            .ok_or(Error::NotFound)
    }

    async fn get_by_name(&self, label: L, name: &ResourceName) -> Result<Object<L>> {
        self.state
            .lock()
            .unwrap()
            .objects
            .values()
            .find(|o| o.label == label && &o.name == name)
            .cloned()
            .ok_or(Error::NotFound)
    }

    async fn list(
        &self,
        label: L,
        namespace: Option<&ResourceName>,
        max_results: Option<usize>,
        page_token: Option<String>,
    ) -> Result<(Vec<Object<L>>, Option<String>)> {
        op_list_objects(
            &self.state.lock().unwrap(),
            label,
            namespace,
            max_results,
            page_token,
        )
    }
}

#[async_trait::async_trait]
impl<L: Label> ObjectStore<L> for InMemoryStore<L> {
    async fn create(
        &self,
        label: L,
        name: &ResourceName,
        properties: Option<serde_json::Value>,
        id: Option<Uuid>,
    ) -> Result<Object<L>> {
        op_create(&mut self.state.lock().unwrap(), label, name, properties, id)
    }

    async fn update(
        &self,
        id: &Uuid,
        properties: Option<serde_json::Value>,
        precondition: Precondition,
    ) -> Result<Object<L>> {
        op_update(
            &mut self.state.lock().unwrap(),
            id,
            properties,
            precondition,
        )
    }

    async fn rename(
        &self,
        id: &Uuid,
        new_name: &ResourceName,
        precondition: Precondition,
    ) -> Result<Object<L>> {
        op_rename(&mut self.state.lock().unwrap(), id, new_name, precondition)
    }

    async fn delete(&self, id: &Uuid) -> Result<()> {
        op_delete(&mut self.state.lock().unwrap(), id)
    }
}

#[async_trait::async_trait]
impl<L: Label> AssociationStoreReader<L> for InMemoryStore<L> {
    async fn list(
        &self,
        from_id: Uuid,
        label: &str,
        target_label: Option<L>,
        max_results: Option<usize>,
        page_token: Option<String>,
    ) -> Result<(Vec<Association<L>>, Option<String>)> {
        op_list_edges(
            &self.state.lock().unwrap(),
            from_id,
            label,
            target_label,
            max_results,
            page_token,
        )
    }
}

#[async_trait::async_trait]
impl<L: Label> AssociationStore<L> for InMemoryStore<L> {
    async fn add(
        &self,
        from_id: Uuid,
        to_id: Uuid,
        label: &str,
        properties: Option<serde_json::Value>,
    ) -> Result<()> {
        op_add_edge(
            &mut self.state.lock().unwrap(),
            from_id,
            to_id,
            label,
            properties,
            &self.inverse,
        )
    }

    async fn remove(&self, from_id: Uuid, to_id: Uuid, label: &str) -> Result<()> {
        op_remove_edge(
            &mut self.state.lock().unwrap(),
            from_id,
            to_id,
            label,
            &self.inverse,
        )
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
    async fn get(&self, id: &Uuid) -> Result<Object<L>> {
        self.with_state(|s| s.objects.get(id).cloned().ok_or(Error::NotFound))
    }

    async fn get_by_name(&self, label: L, name: &ResourceName) -> Result<Object<L>> {
        self.with_state(|s| {
            s.objects
                .values()
                .find(|o| o.label == label && &o.name == name)
                .cloned()
                .ok_or(Error::NotFound)
        })
    }

    async fn list(
        &self,
        label: L,
        namespace: Option<&ResourceName>,
        max_results: Option<usize>,
        page_token: Option<String>,
    ) -> Result<(Vec<Object<L>>, Option<String>)> {
        self.with_state(|s| op_list_objects(s, label, namespace, max_results, page_token))
    }
}

#[async_trait::async_trait]
impl<L: Label> ObjectStore<L> for InMemoryTx<L> {
    async fn create(
        &self,
        label: L,
        name: &ResourceName,
        properties: Option<serde_json::Value>,
        id: Option<Uuid>,
    ) -> Result<Object<L>> {
        self.with_state(|s| op_create(s, label, name, properties, id))
    }

    async fn update(
        &self,
        id: &Uuid,
        properties: Option<serde_json::Value>,
        precondition: Precondition,
    ) -> Result<Object<L>> {
        self.with_state(|s| op_update(s, id, properties, precondition))
    }

    async fn rename(
        &self,
        id: &Uuid,
        new_name: &ResourceName,
        precondition: Precondition,
    ) -> Result<Object<L>> {
        self.with_state(|s| op_rename(s, id, new_name, precondition))
    }

    async fn delete(&self, id: &Uuid) -> Result<()> {
        self.with_state(|s| op_delete(s, id))
    }
}

#[async_trait::async_trait]
impl<L: Label> AssociationStoreReader<L> for InMemoryTx<L> {
    async fn list(
        &self,
        from_id: Uuid,
        label: &str,
        target_label: Option<L>,
        max_results: Option<usize>,
        page_token: Option<String>,
    ) -> Result<(Vec<Association<L>>, Option<String>)> {
        self.with_state(|s| op_list_edges(s, from_id, label, target_label, max_results, page_token))
    }
}

#[async_trait::async_trait]
impl<L: Label> AssociationStore<L> for InMemoryTx<L> {
    async fn add(
        &self,
        from_id: Uuid,
        to_id: Uuid,
        label: &str,
        properties: Option<serde_json::Value>,
    ) -> Result<()> {
        let inverse = self.store.inverse.clone();
        self.with_state(|s| op_add_edge(s, from_id, to_id, label, properties, &inverse))
    }

    async fn remove(&self, from_id: Uuid, to_id: Uuid, label: &str) -> Result<()> {
        let inverse = self.store.inverse.clone();
        self.with_state(|s| op_remove_edge(s, from_id, to_id, label, &inverse))
    }
}

#[async_trait::async_trait]
impl<L: Label> StoreTx<L> for InMemoryTx<L> {
    async fn commit(self: Box<Self>) -> Result<()> {
        if let Some(working) = self.working.lock().unwrap().take() {
            *self.store.state.lock().unwrap() = working;
        }
        Ok(())
    }

    async fn rollback(self: Box<Self>) -> Result<()> {
        *self.working.lock().unwrap() = None;
        Ok(())
    }
}

#[async_trait::async_trait]
impl<L: Label> Transactional<L> for InMemoryStore<L> {
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
                *tx.working.lock().unwrap() = None;
                Err(e)
            }
        }
    }

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
    async fn cas_update_detects_conflict() {
        let store = InMemoryStore::<Kind>::new();
        let obj = store
            .create(Kind::Node, &rn("a"), None, None)
            .await
            .unwrap();
        assert_eq!(obj.version, 0);

        // Fresh version succeeds and bumps.
        let updated = store
            .update(&obj.id, None, Precondition::Version(0))
            .await
            .unwrap();
        assert_eq!(updated.version, 1);

        // Stale version conflicts.
        let err = store
            .update(&obj.id, None, Precondition::Version(0))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Conflict));

        // `Any` still works unconditionally.
        let again = store
            .update(&obj.id, None, Precondition::Any)
            .await
            .unwrap();
        assert_eq!(again.version, 2);
    }

    #[tokio::test]
    async fn rename_preserves_id_and_associations_and_rejects_collision() {
        let store = InMemoryStore::<Kind>::new();
        let a = store
            .create(Kind::Node, &rn("a"), None, None)
            .await
            .unwrap();
        let b = store
            .create(Kind::Node, &rn("b"), None, None)
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
        let (edges, _) = AssociationStoreReader::list(&store, a.id, "link", None, None, None)
            .await
            .unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to_id, b.id);
    }

    #[tokio::test]
    async fn transaction_rolls_back_on_err() {
        let store = InMemoryStore::<Kind>::new();
        let seed = store
            .create(Kind::Node, &rn("seed"), None, None)
            .await
            .unwrap();

        let store2 = store.clone();
        let seed_id = seed.id;
        let res: Result<()> = store
            .transaction(Box::new(move |tx| {
                Box::pin(async move {
                    tx.delete(&seed_id).await?;
                    tx.create(Kind::Node, &rn("new"), None, None).await?;
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
                    let a = tx.create(Kind::Node, &rn("x"), None, None).await?;
                    let b = tx.create(Kind::Node, &rn("y"), None, None).await?;
                    tx.add(a.id, b.id, "e", None).await?;
                    Ok(a.id)
                })
            }))
            .await;
        let a_id = res.unwrap();
        assert!(store.get(&a_id).await.is_ok());
        let (edges, _) = AssociationStoreReader::list(&store, a_id, "e", None, None, None)
            .await
            .unwrap();
        assert_eq!(edges.len(), 1);
    }

    #[tokio::test]
    async fn inverse_edges_maintained() {
        let store = InMemoryStore::<Kind>::with_inverse(|l| match l {
            "parent_of" => Some("child_of".to_string()),
            _ => None,
        });
        let p = store
            .create(Kind::Node, &rn("p"), None, None)
            .await
            .unwrap();
        let c = store
            .create(Kind::Node, &rn("c"), None, None)
            .await
            .unwrap();

        store.add(p.id, c.id, "parent_of", None).await.unwrap();
        // Forward edge present.
        let (fwd, _) = AssociationStoreReader::list(&store, p.id, "parent_of", None, None, None)
            .await
            .unwrap();
        assert_eq!(fwd.len(), 1);
        // Inverse edge present.
        let (inv, _) = AssociationStoreReader::list(&store, c.id, "child_of", None, None, None)
            .await
            .unwrap();
        assert_eq!(inv.len(), 1);
        assert_eq!(inv[0].to_id, p.id);

        // Removing the forward edge removes the inverse too.
        store.remove(p.id, c.id, "parent_of").await.unwrap();
        let (inv, _) = AssociationStoreReader::list(&store, c.id, "child_of", None, None, None)
            .await
            .unwrap();
        assert!(inv.is_empty());
    }

    #[tokio::test]
    async fn list_paginates_deterministically() {
        let store = InMemoryStore::<Kind>::new();
        for i in 0..5 {
            store
                .create(Kind::Node, &rn(&format!("n{i}")), None, None)
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
