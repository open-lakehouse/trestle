//! Shared conformance battery for store backends.
//!
//! A set of backend-agnostic `async` checks that any implementation of the store
//! traits ([`ObjectStore`], [`AssociationStore`], and [`Transactional`]) should
//! pass. They cover compare-and-swap conflicts, rename identity and association
//! preservation, transaction atomicity, and inverse-edge consistency.
//!
//! Each check takes a **fresh, empty** store (or, for inverse edges, a store
//! configured to maintain the `parent_of` ↔ `child_of` inverse pair). Backends
//! wire them up through [`run_all`], passing a factory. The bundled
//! [`InMemoryStore`](crate::InMemoryStore) and the sqlx `SqlStore` both run this
//! battery.
//!
//! ```
//! use olai_store::conformance::{self, ConformanceLabel};
//! use olai_store::InMemoryStore;
//!
//! # async fn run() {
//! conformance::run_all(
//!     || InMemoryStore::<ConformanceLabel>::new(),
//!     |resolver| InMemoryStore::<ConformanceLabel>::with_inverse(resolver),
//! )
//! .await;
//! # }
//! ```
//!
//! [`ObjectStore`]: crate::ObjectStore
//! [`AssociationStore`]: crate::AssociationStore
//! [`Transactional`]: crate::Transactional

use std::fmt;
use std::str::FromStr;

use crate::name::ResourceName;
use crate::store::{
    AssociationStoreReader, ObjectStoreReader, Precondition, StoreExec, Transactional,
};
use crate::{Error, Label};

/// A minimal single-variant [`Label`] the conformance checks operate over.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct ConformanceLabel;

impl fmt::Display for ConformanceLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("node")
    }
}

impl FromStr for ConformanceLabel {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "node" => Ok(ConformanceLabel),
            other => Err(format!("unknown label: {other}")),
        }
    }
}

impl Label for ConformanceLabel {
    fn as_str(&self) -> &str {
        "node"
    }
}

/// The inverse resolver used by [`inverse_edges`]: `parent_of` ↔ `child_of`.
pub fn parent_child_inverse(label: &str) -> Option<String> {
    match label {
        "parent_of" => Some("child_of".to_string()),
        "child_of" => Some("parent_of".to_string()),
        _ => None,
    }
}

fn rn(s: &str) -> ResourceName {
    ResourceName::from_naive_str_split(s)
}

/// Compare-and-swap: fresh version succeeds and bumps; stale version conflicts;
/// `Any` is unconditional.
pub async fn cas_update<S: StoreExec<ConformanceLabel>>(store: &S) {
    let obj = store
        .create(ConformanceLabel, &rn("a"), None, None, None)
        .await
        .unwrap();
    assert_eq!(obj.version, 0, "fresh object starts at version 0");

    let updated = store
        .update(&obj.id, None, Precondition::Version(0), None)
        .await
        .unwrap();
    assert_eq!(updated.version, 1, "successful CAS bumps the version");

    let err = store
        .update(&obj.id, None, Precondition::Version(0), None)
        .await
        .unwrap_err();
    assert!(
        matches!(err, Error::Conflict),
        "stale version must yield Conflict, got {err:?}"
    );

    let again = store
        .update(&obj.id, None, Precondition::Any, None)
        .await
        .unwrap();
    assert_eq!(again.version, 2, "Any overwrites unconditionally");
}

/// Rename preserves id and associations, honors CAS, and rejects name
/// collisions.
pub async fn rename_semantics<S: StoreExec<ConformanceLabel>>(store: &S) {
    let a = store
        .create(ConformanceLabel, &rn("a"), None, None, None)
        .await
        .unwrap();
    let b = store
        .create(ConformanceLabel, &rn("b"), None, None, None)
        .await
        .unwrap();
    store.add(a.id, b.id, "link", None).await.unwrap();

    let err = store
        .rename(&a.id, &rn("b"), Precondition::Any)
        .await
        .unwrap_err();
    assert!(
        matches!(err, Error::AlreadyExists),
        "rename onto an existing name must collide, got {err:?}"
    );

    let renamed = store
        .rename(&a.id, &rn("a2"), Precondition::Version(a.version))
        .await
        .unwrap();
    assert_eq!(renamed.id, a.id, "rename preserves id");
    assert_eq!(renamed.name, rn("a2"));
    assert!(renamed.version > a.version, "rename bumps the version");

    let (edges, _) = AssociationStoreReader::list(store, a.id, "link", None, None, None)
        .await
        .unwrap();
    assert_eq!(edges.len(), 1, "rename preserves outgoing associations");
    assert_eq!(edges[0].to_id, b.id);
}

/// A transaction that errors leaves no partial writes.
pub async fn transaction_atomicity<
    S: StoreExec<ConformanceLabel> + Transactional<ConformanceLabel>,
>(
    store: &S,
) {
    let seed = store
        .create(ConformanceLabel, &rn("seed"), None, None, None)
        .await
        .unwrap();
    let seed_id = seed.id;

    let res: crate::Result<()> = store
        .transaction(Box::new(move |tx| {
            Box::pin(async move {
                tx.delete(&seed_id).await?;
                tx.create(ConformanceLabel, &rn("new"), None, None, None)
                    .await?;
                Err(Error::generic("boom"))
            })
        }))
        .await;
    assert!(res.is_err(), "the closure error must propagate");

    assert!(
        store.get(&seed_id).await.is_ok(),
        "rollback must restore the deleted seed"
    );
    assert!(
        store
            .get_by_name(ConformanceLabel, &rn("new"))
            .await
            .is_err(),
        "rollback must discard the created object"
    );
}

/// A transaction that succeeds commits all of its writes atomically.
pub async fn transaction_commit<
    S: StoreExec<ConformanceLabel> + Transactional<ConformanceLabel>,
>(
    store: &S,
) {
    let res: crate::Result<uuid::Uuid> = store
        .transaction(Box::new(|tx| {
            Box::pin(async move {
                let a = tx
                    .create(ConformanceLabel, &rn("x"), None, None, None)
                    .await?;
                let b = tx
                    .create(ConformanceLabel, &rn("y"), None, None, None)
                    .await?;
                tx.add(a.id, b.id, "e", None).await?;
                Ok(a.id)
            })
        }))
        .await;
    let a_id = res.unwrap();
    assert!(
        store.get(&a_id).await.is_ok(),
        "committed object must persist"
    );
    let (edges, _) = AssociationStoreReader::list(store, a_id, "e", None, None, None)
        .await
        .unwrap();
    assert_eq!(edges.len(), 1, "committed edge must persist");
}

/// Adding/removing an edge with an inverse maintains the inverse edge in step.
///
/// `store` must be configured with [`parent_child_inverse`].
pub async fn inverse_edges<S: StoreExec<ConformanceLabel>>(store: &S) {
    let p = store
        .create(ConformanceLabel, &rn("p"), None, None, None)
        .await
        .unwrap();
    let c = store
        .create(ConformanceLabel, &rn("c"), None, None, None)
        .await
        .unwrap();

    store.add(p.id, c.id, "parent_of", None).await.unwrap();
    let (fwd, _) = AssociationStoreReader::list(store, p.id, "parent_of", None, None, None)
        .await
        .unwrap();
    assert_eq!(fwd.len(), 1, "forward edge present");
    let (inv, _) = AssociationStoreReader::list(store, c.id, "child_of", None, None, None)
        .await
        .unwrap();
    assert_eq!(inv.len(), 1, "inverse edge maintained on add");
    assert_eq!(inv[0].to_id, p.id);

    store.remove(p.id, c.id, "parent_of").await.unwrap();
    let (inv, _) = AssociationStoreReader::list(store, c.id, "child_of", None, None, None)
        .await
        .unwrap();
    assert!(inv.is_empty(), "inverse edge removed on remove");
}

/// The opaque `sensitive` blob is written atomically with the object, read back only through
/// [`get_sensitive`](ObjectStoreReader::get_sensitive), preserved across an update that omits it,
/// replaced when supplied, and dropped with the object.
///
/// This exercises the store seam directly with plain bytes — no encryption — so both backends
/// prove they persist and return the blob without leaking it into the ordinary read path.
pub async fn sensitive_blob_roundtrip<S: StoreExec<ConformanceLabel>>(store: &S) {
    use bytes::Bytes;

    let blob = Bytes::from_static(b"opaque-sealed-bytes");
    let obj = store
        .create(ConformanceLabel, &rn("s"), None, None, Some(blob.clone()))
        .await
        .unwrap();

    // The blob never rides the ordinary read path.
    let got = ObjectStoreReader::get(store, &obj.id).await.unwrap();
    assert!(
        got.properties.is_none(),
        "sensitive blob must not surface in properties"
    );
    // ...but is retrievable through the dedicated accessor.
    assert_eq!(
        store.get_sensitive(&obj.id).await.unwrap().as_deref(),
        Some(&blob[..]),
        "stored blob must round-trip"
    );

    // An update that omits the blob preserves it.
    store
        .update(&obj.id, None, Precondition::Any, None)
        .await
        .unwrap();
    assert_eq!(
        store.get_sensitive(&obj.id).await.unwrap().as_deref(),
        Some(&blob[..]),
        "update without a blob must preserve the existing one"
    );

    // An update that supplies a blob replaces it.
    let blob2 = Bytes::from_static(b"replacement");
    store
        .update(&obj.id, None, Precondition::Any, Some(blob2.clone()))
        .await
        .unwrap();
    assert_eq!(
        store.get_sensitive(&obj.id).await.unwrap().as_deref(),
        Some(&blob2[..]),
        "supplying a blob must replace the existing one"
    );

    // Deleting the object drops the blob with it.
    store.delete(&obj.id).await.unwrap();
    assert!(
        matches!(
            store.get_sensitive(&obj.id).await,
            Ok(None) | Err(Error::NotFound)
        ),
        "blob must be gone once the object is deleted"
    );
}

/// Run the entire battery.
///
/// `fresh` builds a new empty store; `with_inverse` builds one that maintains
/// the `parent_of` ↔ `child_of` pair (pass [`parent_child_inverse`] through to
/// the backend). Each check gets its own fresh store.
pub async fn run_all<S, F, G>(fresh: F, with_inverse: G)
where
    S: StoreExec<ConformanceLabel> + Transactional<ConformanceLabel>,
    F: Fn() -> S,
    G: Fn(fn(&str) -> Option<String>) -> S,
{
    cas_update(&fresh()).await;
    rename_semantics(&fresh()).await;
    transaction_atomicity(&fresh()).await;
    transaction_commit(&fresh()).await;
    sensitive_blob_roundtrip(&fresh()).await;
    inverse_edges(&with_inverse(parent_child_inverse)).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::InMemoryStore;

    #[tokio::test]
    async fn in_memory_store_passes_conformance() {
        run_all(
            InMemoryStore::<ConformanceLabel>::new,
            InMemoryStore::<ConformanceLabel>::with_inverse,
        )
        .await;
    }
}
