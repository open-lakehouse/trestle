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

use crate::filter::Filter;
use crate::name::ResourceName;
use crate::store::{
    EdgeEndpoint, EdgeQuery, ObjectStoreReader, Precondition, StoreExec, Transactional,
};
use crate::{Error, Label};

/// A minimal [`Label`] the conformance checks operate over.
///
/// Two variants so target-label filtering has something to discriminate on;
/// [`Node`](Self::Node) is the default used by every check that doesn't care about kind.
#[derive(Debug, Clone, Copy, Default, Hash, PartialEq, Eq)]
pub enum ConformanceLabel {
    /// The default object kind.
    #[default]
    Node,
    /// A second kind, used only to exercise `target_label` filtering.
    Other,
}

impl fmt::Display for ConformanceLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ConformanceLabel {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "node" => Ok(ConformanceLabel::Node),
            "other" => Ok(ConformanceLabel::Other),
            other => Err(format!("unknown label: {other}")),
        }
    }
}

impl Label for ConformanceLabel {
    fn as_str(&self) -> &str {
        match self {
            ConformanceLabel::Node => "node",
            ConformanceLabel::Other => "other",
        }
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
        .create(ConformanceLabel::Node, &rn("a"), None, None, None)
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
        .create(ConformanceLabel::Node, &rn("a"), None, None, None)
        .await
        .unwrap();
    let b = store
        .create(ConformanceLabel::Node, &rn("b"), None, None, None)
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

    let (edges, _) = store
        .query_edges(EdgeQuery::from(a.id, "link"))
        .await
        .unwrap();
    assert_eq!(edges.len(), 1, "rename preserves outgoing associations");
    assert_eq!(edges[0].to_id, b.id);
}

/// Object names fold ASCII case: a name differing only in case is the same
/// object. `get_by_name` finds it, `create` collides with it, and namespace
/// prefix listing matches it — consistently across backends.
pub async fn case_insensitive_names<S: StoreExec<ConformanceLabel>>(store: &S) {
    let created = store
        .create(ConformanceLabel::Node, &rn("Catalog"), None, None, None)
        .await
        .unwrap();

    // get_by_name folds case.
    let by_lower = store
        .get_by_name(ConformanceLabel::Node, &rn("catalog"))
        .await
        .unwrap();
    assert_eq!(by_lower.id, created.id, "get_by_name must fold ASCII case");
    let by_upper = store
        .get_by_name(ConformanceLabel::Node, &rn("CATALOG"))
        .await
        .unwrap();
    assert_eq!(by_upper.id, created.id, "get_by_name must fold ASCII case");

    // create collides with a case-variant of an existing name.
    let err = store
        .create(ConformanceLabel::Node, &rn("catalog"), None, None, None)
        .await
        .unwrap_err();
    assert!(
        matches!(err, Error::AlreadyExists),
        "creating a case-variant of an existing name must collide, got {err:?}"
    );

    // Namespace prefix listing folds case: a child under `Catalog.Schema`
    // is listed when scoping to the differently-cased `catalog.schema`.
    let child = store
        .create(
            ConformanceLabel::Node,
            &rn("Catalog.Schema.table"),
            None,
            None,
            None,
        )
        .await
        .unwrap();
    let (listed, _) = store
        .list(
            ConformanceLabel::Node,
            Some(&rn("catalog.schema")),
            None,
            None,
        )
        .await
        .unwrap();
    assert!(
        listed.iter().any(|o| o.id == child.id),
        "namespace prefix listing must fold ASCII case"
    );
}

/// A transaction that errors leaves no partial writes.
pub async fn transaction_atomicity<
    S: StoreExec<ConformanceLabel> + Transactional<ConformanceLabel>,
>(
    store: &S,
) {
    let seed = store
        .create(ConformanceLabel::Node, &rn("seed"), None, None, None)
        .await
        .unwrap();
    let seed_id = seed.id;

    let res: crate::Result<()> = store
        .transaction(Box::new(move |tx| {
            Box::pin(async move {
                tx.delete(&seed_id).await?;
                tx.create(ConformanceLabel::Node, &rn("new"), None, None, None)
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
            .get_by_name(ConformanceLabel::Node, &rn("new"))
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
                    .create(ConformanceLabel::Node, &rn("x"), None, None, None)
                    .await?;
                let b = tx
                    .create(ConformanceLabel::Node, &rn("y"), None, None, None)
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
    let (edges, _) = store.query_edges(EdgeQuery::from(a_id, "e")).await.unwrap();
    assert_eq!(edges.len(), 1, "committed edge must persist");
}

/// Adding/removing an edge with an inverse maintains the inverse edge in step.
///
/// `store` must be configured with [`parent_child_inverse`].
pub async fn inverse_edges<S: StoreExec<ConformanceLabel>>(store: &S) {
    let p = store
        .create(ConformanceLabel::Node, &rn("p"), None, None, None)
        .await
        .unwrap();
    let c = store
        .create(ConformanceLabel::Node, &rn("c"), None, None, None)
        .await
        .unwrap();

    store.add(p.id, c.id, "parent_of", None).await.unwrap();
    let (fwd, _) = store
        .query_edges(EdgeQuery::from(p.id, "parent_of"))
        .await
        .unwrap();
    assert_eq!(fwd.len(), 1, "forward edge present");
    let (inv, _) = store
        .query_edges(EdgeQuery::from(c.id, "child_of"))
        .await
        .unwrap();
    assert_eq!(inv.len(), 1, "inverse edge maintained on add");
    assert_eq!(inv[0].to_id, p.id);

    store.remove(p.id, c.id, "parent_of").await.unwrap();
    let (inv, _) = store
        .query_edges(EdgeQuery::from(c.id, "child_of"))
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
        .create(
            ConformanceLabel::Node,
            &rn("s"),
            None,
            None,
            Some(blob.clone()),
        )
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

    // `set_sensitive` rewrites only the blob, leaving the object's version untouched.
    let before = ObjectStoreReader::get(store, &obj.id).await.unwrap();
    let blob3 = Bytes::from_static(b"rewrapped");
    store.set_sensitive(&obj.id, blob3.clone()).await.unwrap();
    assert_eq!(
        store.get_sensitive(&obj.id).await.unwrap().as_deref(),
        Some(&blob3[..]),
        "set_sensitive must replace the blob"
    );
    let after = ObjectStoreReader::get(store, &obj.id).await.unwrap();
    assert_eq!(
        after.version, before.version,
        "set_sensitive must not bump the object version"
    );

    // Deleting the object drops the blob with it; a missing object reads as `Ok(None)`.
    store.delete(&obj.id).await.unwrap();
    assert!(
        matches!(store.get_sensitive(&obj.id).await, Ok(None)),
        "blob must be gone (Ok(None)) once the object is deleted"
    );
}

/// Searching objects by payload matches exactly the set the reference
/// [`Filter::matches`] selects, for each operator and boolean composition.
///
/// The store's [`search`](ObjectStoreReader::search) result is checked against the payloads
/// filtered directly through `Filter::matches`, binding any backend (including a pushdown
/// backend) to the reference evaluator as the source of truth.
pub async fn search_object_predicates<S: StoreExec<ConformanceLabel>>(store: &S) {
    let payloads = [
        serde_json::json!({ "owner": "alice", "size": 10, "tags": ["a", "b"] }),
        serde_json::json!({ "owner": "bob", "size": 20, "tags": ["b", "c"] }),
        serde_json::json!({ "owner": "carol", "size": 30, "archived": null }),
        serde_json::json!({ "owner": "alice", "size": 40, "tags": [] }),
    ];
    for (i, p) in payloads.iter().enumerate() {
        store
            .create(
                ConformanceLabel::Node,
                &rn(&format!("o{i}")),
                Some(p.clone()),
                None,
                None,
            )
            .await
            .unwrap();
    }

    // For a given filter, the store's search must return exactly the objects whose payloads
    // the reference evaluator selects (compared as an id set, order-independent).
    let check = async |filter: Filter| {
        let (hits, token) = store
            .search(ConformanceLabel::Node, None, &filter, None, None)
            .await
            .unwrap();
        assert!(token.is_none(), "unbounded search returns a single page");
        let mut got: Vec<_> = hits.iter().map(|o| o.properties.clone().unwrap()).collect();
        let mut want: Vec<_> = payloads
            .iter()
            .filter(|p| filter.matches(p))
            .cloned()
            .collect();
        got.sort_by_key(|v| v.to_string());
        want.sort_by_key(|v| v.to_string());
        assert_eq!(
            got, want,
            "search disagreed with Filter::matches for {filter:?}"
        );
    };

    check(Filter::eq("owner", "alice")).await;
    check(Filter::ne("owner", "alice")).await;
    check(Filter::gt("size", 20)).await;
    check(Filter::ge("size", 20)).await;
    check(Filter::lt("size", 20)).await;
    check(Filter::le("size", 20)).await;
    check(Filter::contains("tags", "b")).await;
    check(Filter::exists("archived")).await;
    check(Filter::exists("tags")).await;
    check(Filter::all([
        Filter::eq("owner", "alice"),
        Filter::gt("size", 15),
    ]))
    .await;
    check(Filter::any([
        Filter::eq("owner", "bob"),
        Filter::eq("owner", "carol"),
    ]))
    .await;
    check(Filter::eq("owner", "alice").negate()).await;
    // A predicate no object matches, and one every object matches.
    check(Filter::eq("owner", "nobody")).await;
    check(Filter::all([])).await;
}

/// Paging a filtered object search returns every match exactly once even when matching and
/// non-matching payloads are interleaved — the filter must never run behind a `LIMIT` that
/// could truncate matches (the search analogue of
/// [`namespace_filtered_listing_pages_completely`]).
///
/// [`namespace_filtered_listing_pages_completely`]: crate::backend
pub async fn search_object_pagination_filters_completely<S: StoreExec<ConformanceLabel>>(
    store: &S,
) {
    // Interleave matching (keep=true) and non-matching (keep=false) payloads so a naive
    // "SQL LIMIT then filter" would lose matches and desync the page token.
    for i in 0..6 {
        for keep in [true, false] {
            let p = serde_json::json!({ "keep": keep, "i": i });
            let name = rn(&format!("k{keep}i{i}"));
            store
                .create(ConformanceLabel::Node, &name, Some(p), None, None)
                .await
                .unwrap();
        }
    }

    let filter = Filter::eq("keep", true);
    let mut seen = Vec::new();
    let mut token = None;
    loop {
        let (page, next) = store
            .search(ConformanceLabel::Node, None, &filter, Some(2), token)
            .await
            .unwrap();
        assert!(page.len() <= 2, "respects max_results");
        assert!(
            page.iter()
                .all(|o| o.properties.as_ref().unwrap()["keep"] == true),
            "every returned object matches the filter"
        );
        seen.extend(page.into_iter().map(|o| o.id));
        match next {
            Some(t) => token = Some(t),
            None => break,
        }
    }
    assert_eq!(seen.len(), 6, "every matching object must be paged");
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), 6, "no duplicates across pages");
}

/// A namespace prefix and a payload filter compose: only objects under the namespace *and*
/// matching the filter are returned, and paging still drains completely.
pub async fn search_namespace_and_filter<S: StoreExec<ConformanceLabel>>(store: &S) {
    // Under "ns": three match the filter, two don't. Under "other": one matches — must be
    // excluded by the namespace.
    for (name, active) in [
        ("ns.a", true),
        ("ns.b", true),
        ("ns.c", true),
        ("ns.d", false),
        ("ns.e", false),
        ("other.f", true),
    ] {
        let p = serde_json::json!({ "active": active });
        store
            .create(ConformanceLabel::Node, &rn(name), Some(p), None, None)
            .await
            .unwrap();
    }

    let ns = rn("ns");
    let filter = Filter::eq("active", true);
    let mut seen = Vec::new();
    let mut token = None;
    loop {
        let (page, next) = store
            .search(ConformanceLabel::Node, Some(&ns), &filter, Some(2), token)
            .await
            .unwrap();
        for o in &page {
            assert!(o.name.prefix_matches(&ns), "namespace prefix honored");
            assert_eq!(o.properties.as_ref().unwrap()["active"], true);
        }
        seen.extend(page.into_iter().map(|o| o.id));
        match next {
            Some(t) => token = Some(t),
            None => break,
        }
    }
    assert_eq!(
        seen.len(),
        3,
        "only ns.* objects matching the filter, all of them"
    );
}

/// Querying edges with a payload filter matches the set the reference [`Filter::matches`] selects.
pub async fn edge_filter_predicates<S: StoreExec<ConformanceLabel>>(store: &S) {
    let src = store
        .create(ConformanceLabel::Node, &rn("src"), None, None, None)
        .await
        .unwrap();
    let edges = [
        serde_json::json!({ "weight": 1, "kind": "x" }),
        serde_json::json!({ "weight": 5, "kind": "y" }),
        serde_json::json!({ "weight": 9, "kind": "x" }),
    ];
    for (i, e) in edges.iter().enumerate() {
        let dst = store
            .create(
                ConformanceLabel::Node,
                &rn(&format!("dst{i}")),
                None,
                None,
                None,
            )
            .await
            .unwrap();
        store
            .add(src.id, dst.id, "link", Some(e.clone()))
            .await
            .unwrap();
    }

    let check = async |filter: Filter| {
        let (hits, token) = store
            .query_edges(EdgeQuery::from(src.id, "link").filter(&filter))
            .await
            .unwrap();
        assert!(token.is_none());
        let mut got: Vec<_> = hits.iter().map(|a| a.properties.clone().unwrap()).collect();
        let mut want: Vec<_> = edges
            .iter()
            .filter(|e| filter.matches(e))
            .cloned()
            .collect();
        got.sort_by_key(|v| v.to_string());
        want.sort_by_key(|v| v.to_string());
        assert_eq!(
            got, want,
            "query_edges disagreed with Filter::matches for {filter:?}"
        );
    };

    check(Filter::eq("kind", "x")).await;
    check(Filter::gt("weight", 4)).await;
    check(Filter::all([
        Filter::eq("kind", "x"),
        Filter::lt("weight", 5),
    ]))
    .await;
    check(Filter::exists("kind")).await;
}

/// Paging a filtered edge query returns every match exactly once even when matching and
/// non-matching edges are interleaved.
pub async fn edge_filter_pagination_completes<S: StoreExec<ConformanceLabel>>(store: &S) {
    let src = store
        .create(ConformanceLabel::Node, &rn("src"), None, None, None)
        .await
        .unwrap();
    for i in 0..6 {
        for keep in [true, false] {
            let dst = store
                .create(
                    ConformanceLabel::Node,
                    &rn(&format!("d{keep}{i}")),
                    None,
                    None,
                    None,
                )
                .await
                .unwrap();
            let p = serde_json::json!({ "keep": keep });
            store.add(src.id, dst.id, "link", Some(p)).await.unwrap();
        }
    }

    let filter = Filter::eq("keep", true);
    let mut seen = Vec::new();
    let mut token = None;
    loop {
        let (page, next) = store
            .query_edges(
                EdgeQuery::from(src.id, "link")
                    .filter(&filter)
                    .page(Some(2), token),
            )
            .await
            .unwrap();
        assert!(page.len() <= 2);
        assert!(
            page.iter()
                .all(|a| a.properties.as_ref().unwrap()["keep"] == true)
        );
        seen.extend(page.into_iter().map(|a| a.id));
        match next {
            Some(t) => token = Some(t),
            None => break,
        }
    }
    assert_eq!(seen.len(), 6, "every matching edge must be paged");
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), 6, "no duplicates across pages");
}

/// The predicates a SQL backend cannot push faithfully — `Contains`, `Ne`, comparisons whose
/// value is null/array/object, and any composite that includes one — still match the reference
/// [`Filter::matches`] semantics, because the backend falls back to Rust-side filtering.
///
/// This guards the fallback path specifically: a pushdown backend must produce the same result
/// as the in-memory backend for every filter here.
pub async fn search_fallback_predicates_agree<S: StoreExec<ConformanceLabel>>(store: &S) {
    let payloads = [
        serde_json::json!({ "tags": ["red", "blue"], "state": "active", "nick": null, "a.b": 1 }),
        serde_json::json!({ "tags": ["green"], "state": "archived" }),
        serde_json::json!({ "tags": [], "state": "active", "count": 3 }),
        serde_json::json!({ "note": "hello world", "state": "active", "a.b": 2 }),
    ];
    for (i, p) in payloads.iter().enumerate() {
        store
            .create(
                ConformanceLabel::Node,
                &rn(&format!("f{i}")),
                Some(p.clone()),
                None,
                None,
            )
            .await
            .unwrap();
    }

    let check = async |filter: Filter| {
        let (hits, _) = store
            .search(ConformanceLabel::Node, None, &filter, None, None)
            .await
            .unwrap();
        let mut got: Vec<_> = hits.iter().map(|o| o.properties.clone().unwrap()).collect();
        let mut want: Vec<_> = payloads
            .iter()
            .filter(|p| filter.matches(p))
            .cloned()
            .collect();
        got.sort_by_key(|v| v.to_string());
        want.sort_by_key(|v| v.to_string());
        assert_eq!(
            got, want,
            "fallback search disagreed with Filter::matches for {filter:?}"
        );
    };

    // Contains — array membership and string substring.
    check(Filter::contains("tags", "blue")).await;
    check(Filter::contains("note", "world")).await;
    // Ne — including a missing path (must not match) and a present mismatch.
    check(Filter::ne("state", "active")).await;
    check(Filter::ne("count", 3)).await;
    // Comparisons whose value is not a pushable scalar.
    check(Filter::eq("nick", serde_json::Value::Null)).await;
    check(Filter::eq("tags", serde_json::json!([]))).await;
    // A composite mixing a pushable leaf with a non-pushable one falls back wholesale.
    check(Filter::all([
        Filter::eq("state", "active"),
        Filter::contains("tags", "red"),
    ]))
    .await;
    check(Filter::any([
        Filter::gt("count", 2),
        Filter::ne("state", "archived"),
    ]))
    .await;
    // A field key containing a JSONPath metacharacter: a `$.a.b` path can't be pushed to
    // SQLite (it would parse as nested `a`→`b`), so it must fall back and still match the
    // literal key `"a.b"`.
    check(Filter::eq(crate::filter::FieldPath::new(["a.b"]), 1)).await;
    check(Filter::exists(crate::filter::FieldPath::new(["a.b"]))).await;
}

/// Listing outgoing edges returns them most-recent-first, and paging preserves that order
/// with no gaps or duplicates.
///
/// This is the load-bearing recency check: it forces both backends onto time-ordered
/// (UUIDv7) edge ids, since a random id would not sort by creation time.
pub async fn edge_listing_is_recency_ordered<S: StoreExec<ConformanceLabel>>(store: &S) {
    let src = store
        .create(ConformanceLabel::Node, &rn("src"), None, None, None)
        .await
        .unwrap();
    // Insert edges in a known order; each carries its insertion index in the payload.
    let n = 7usize;
    for i in 0..n {
        let dst = store
            .create(
                ConformanceLabel::Node,
                &rn(&format!("dst{i}")),
                None,
                None,
                None,
            )
            .await
            .unwrap();
        store
            .add(
                src.id,
                dst.id,
                "link",
                Some(serde_json::json!({ "seq": i })),
            )
            .await
            .unwrap();
    }

    // A single unpaged listing is newest-first: seq counts down from n-1 to 0.
    let (all, token) = store
        .query_edges(EdgeQuery::from(src.id, "link"))
        .await
        .unwrap();
    assert!(token.is_none());
    let seqs: Vec<u64> = all
        .iter()
        .map(|e| e.properties.as_ref().unwrap()["seq"].as_u64().unwrap())
        .collect();
    let expected: Vec<u64> = (0..n as u64).rev().collect();
    assert_eq!(seqs, expected, "listing must be most-recent-first");

    // Paging preserves the order and covers every edge exactly once.
    let mut paged = Vec::new();
    let mut tok = None;
    loop {
        let (page, next) = store
            .query_edges(EdgeQuery::from(src.id, "link").page(Some(3), tok))
            .await
            .unwrap();
        assert!(page.len() <= 3);
        paged.extend(
            page.iter()
                .map(|e| e.properties.as_ref().unwrap()["seq"].as_u64().unwrap()),
        );
        match next {
            Some(t) => tok = Some(t),
            None => break,
        }
    }
    assert_eq!(
        paged, expected,
        "paged listing must match the unpaged order"
    );
}

/// A `[since, until)` time window selects exactly the edges created within it, resolved against
/// the time-ordered (UUIDv7) edge id.
///
/// Edges are inserted with a real gap between them so each lands in a distinct millisecond; the
/// timestamp captured *between* two inserts is then a boundary that cleanly partitions them.
pub async fn edge_time_window_selects_range<S: StoreExec<ConformanceLabel>>(store: &S) {
    let src = store
        .create(ConformanceLabel::Node, &rn("src"), None, None, None)
        .await
        .unwrap();

    // Insert 5 edges, recording the instant just before each. A 2ms sleep guarantees the next
    // edge's v7 id lands in a strictly later millisecond than the captured boundary.
    let mut bounds = Vec::new();
    for i in 0..5 {
        bounds.push(chrono::Utc::now());
        std::thread::sleep(std::time::Duration::from_millis(2));
        let dst = store
            .create(
                ConformanceLabel::Node,
                &rn(&format!("dst{i}")),
                None,
                None,
                None,
            )
            .await
            .unwrap();
        store
            .add(
                src.id,
                dst.id,
                "link",
                Some(serde_json::json!({ "seq": i })),
            )
            .await
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
    }

    let seqs = async |q: EdgeQuery<'_, ConformanceLabel>| -> Vec<u64> {
        let (edges, _) = store.query_edges(q).await.unwrap();
        edges
            .iter()
            .map(|e| e.properties.as_ref().unwrap()["seq"].as_u64().unwrap())
            .collect()
    };

    // since = bound before edge 2 → edges 2,3,4 (newest-first: 4,3,2).
    assert_eq!(
        seqs(EdgeQuery::from(src.id, "link").since(bounds[2])).await,
        vec![4, 3, 2],
        "since selects edges created at or after the boundary"
    );

    // until = bound before edge 2 → edges 0,1 (newest-first: 1,0).
    assert_eq!(
        seqs(EdgeQuery::from(src.id, "link").until(bounds[2])).await,
        vec![1, 0],
        "until excludes edges created at or after the boundary"
    );

    // [since edge1, until edge4) → edges 1,2,3 (newest-first: 3,2,1).
    assert_eq!(
        seqs(
            EdgeQuery::from(src.id, "link")
                .since(bounds[1])
                .until(bounds[4])
        )
        .await,
        vec![3, 2, 1],
        "half-open window selects the interior edges"
    );

    // A window before everything is empty; a window after everything is empty.
    assert!(
        seqs(EdgeQuery::from(src.id, "link").until(bounds[0]))
            .await
            .is_empty(),
        "window entirely before the first edge is empty"
    );
}

/// A `target_label` filter returns every matching edge across pages — guarding the historical
/// hazard of filtering the target label *after* a SQL `LIMIT` (which could drop matches).
pub async fn edge_target_label_pages_completely<S: StoreExec<ConformanceLabel>>(store: &S) {
    let src = store
        .create(ConformanceLabel::Node, &rn("src"), None, None, None)
        .await
        .unwrap();
    // Interleave Node- and Other-labelled targets so a naive post-LIMIT filter would drop some.
    let mut want = 0usize;
    for i in 0..12 {
        let label = if i % 2 == 0 {
            ConformanceLabel::Node
        } else {
            ConformanceLabel::Other
        };
        if label == ConformanceLabel::Other {
            want += 1;
        }
        let dst = store
            .create(label, &rn(&format!("dst{i}")), None, None, None)
            .await
            .unwrap();
        store.add(src.id, dst.id, "link", None).await.unwrap();
    }

    let mut seen = Vec::new();
    let mut tok = None;
    loop {
        let (page, next) = store
            .query_edges(
                EdgeQuery::from(src.id, "link")
                    .target_label(ConformanceLabel::Other)
                    .page(Some(2), tok),
            )
            .await
            .unwrap();
        assert!(page.len() <= 2);
        assert!(
            page.iter().all(|e| e.to_label == ConformanceLabel::Other),
            "target_label must be honored"
        );
        seen.extend(page.into_iter().map(|e| e.id));
        match next {
            Some(t) => tok = Some(t),
            None => break,
        }
    }
    seen.sort();
    seen.dedup();
    assert_eq!(
        seen.len(),
        want,
        "every Other-target edge must be paged, none dropped"
    );
}

/// Incoming edges are listed by a reverse scan on the target — independent of any inverse
/// resolver (this store is built without one).
pub async fn incoming_edges_listed<S: StoreExec<ConformanceLabel>>(store: &S) {
    let x = store
        .create(ConformanceLabel::Node, &rn("x"), None, None, None)
        .await
        .unwrap();
    for name in ["a", "b", "c"] {
        let src = store
            .create(ConformanceLabel::Node, &rn(name), None, None, None)
            .await
            .unwrap();
        store.add(src.id, x.id, "link", None).await.unwrap();
    }

    let (incoming, token) = store
        .query_edges(EdgeQuery::into(x.id, "link"))
        .await
        .unwrap();
    assert!(token.is_none());
    assert_eq!(incoming.len(), 3, "all incoming edges listed");
    assert!(
        incoming.iter().all(|e| e.to_id == x.id),
        "incoming edges all point at x"
    );
}

/// A `target_id` restriction returns only the single edge to that specific target.
pub async fn edge_target_id_restriction<S: StoreExec<ConformanceLabel>>(store: &S) {
    let src = store
        .create(ConformanceLabel::Node, &rn("src"), None, None, None)
        .await
        .unwrap();
    let mut targets = Vec::new();
    for i in 0..4 {
        let dst = store
            .create(
                ConformanceLabel::Node,
                &rn(&format!("dst{i}")),
                None,
                None,
                None,
            )
            .await
            .unwrap();
        store.add(src.id, dst.id, "link", None).await.unwrap();
        targets.push(dst.id);
    }

    let (hits, token) = store
        .query_edges(EdgeQuery::from(src.id, "link").target_id(targets[2]))
        .await
        .unwrap();
    assert!(token.is_none());
    assert_eq!(hits.len(), 1, "exactly the one edge to the chosen target");
    assert_eq!(hits[0].to_id, targets[2]);
}

/// `count_edges` agrees with the length of the full listing, with and without a target label.
pub async fn count_edges_matches_list<S: StoreExec<ConformanceLabel>>(store: &S) {
    let src = store
        .create(ConformanceLabel::Node, &rn("src"), None, None, None)
        .await
        .unwrap();
    for i in 0..5 {
        let label = if i < 2 {
            ConformanceLabel::Other
        } else {
            ConformanceLabel::Node
        };
        let dst = store
            .create(label, &rn(&format!("dst{i}")), None, None, None)
            .await
            .unwrap();
        store.add(src.id, dst.id, "link", None).await.unwrap();
    }

    let count = store
        .count_edges(EdgeEndpoint::From(src.id), "link", None)
        .await
        .unwrap();
    assert_eq!(count, 5, "count matches total edges");

    let count_other = store
        .count_edges(
            EdgeEndpoint::From(src.id),
            "link",
            Some(ConformanceLabel::Other),
        )
        .await
        .unwrap();
    assert_eq!(count_other, 2, "count honors target_label");
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
    case_insensitive_names(&fresh()).await;
    transaction_atomicity(&fresh()).await;
    transaction_commit(&fresh()).await;
    sensitive_blob_roundtrip(&fresh()).await;
    inverse_edges(&with_inverse(parent_child_inverse)).await;
    search_object_predicates(&fresh()).await;
    search_object_pagination_filters_completely(&fresh()).await;
    search_namespace_and_filter(&fresh()).await;
    edge_filter_predicates(&fresh()).await;
    edge_filter_pagination_completes(&fresh()).await;
    search_fallback_predicates_agree(&fresh()).await;
    edge_listing_is_recency_ordered(&fresh()).await;
    edge_time_window_selects_range(&fresh()).await;
    edge_target_label_pages_completely(&fresh()).await;
    incoming_edges_listed(&fresh()).await;
    edge_target_id_restriction(&fresh()).await;
    count_edges_matches_list(&fresh()).await;
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
