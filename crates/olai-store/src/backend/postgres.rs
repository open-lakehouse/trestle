//! Postgres-backed store ([`PgStore`]).
//!
//! A native Postgres implementation of [`ObjectStore`], [`AssociationStore`], and
//! [`Transactional`] on top of [`sqlx`] and Postgres. It is the sibling of the
//! [`sqlite`](super::sqlite) backend and passes the same
//! [conformance](crate::conformance) battery, but leans on Postgres-native types
//! and features rather than the portable-TEXT encoding the SQLite backend uses:
//!
//! - `uuid` primary keys (ids bind and decode natively — no string round-trip).
//! - `timestamptz` instants (`DateTime<Utc>` directly — no RFC3339 parsing).
//! - `bigint` versions and a **real** compare-and-swap:
//!   `UPDATE … SET version = version + 1 WHERE id = $1 AND version = $2 RETURNING …`.
//! - `jsonb` property bags with a GIN index, so payload-filter pushdown
//!   ([`search`](ObjectStoreReader::search) / [`query_edges`](AssociationStoreReader::query_edges))
//!   uses `->`/`->>`/`jsonb_typeof` and can exploit containment.
//! - `bytea` for the sealed sensitive blob.
//!
//! Enabled by the `postgres` feature.
//!
//! # Requirements
//!
//! **PostgreSQL 12+ built with ICU** (the official `postgres` Docker images are).
//! The floor is the non-deterministic ICU collation the schema uses for
//! case-insensitive names (`deterministic = false`, added in PG 12); every other
//! feature — `jsonb`, GIN `jsonb_path_ops`, `timestamptz`, `bytea` — is far older.
//! It is **not** driven by UUIDv7 in the schema: ids are minted client-side by
//! the [`uuid`] crate (`Uuid::now_v7` for both objects and edges, so listings
//! are time-ordered) and bound as parameters, so the schema needs neither PG
//! 18's native `uuidv7()` nor a hand-written PL/pgSQL generator.
//!
//! SQL is checked at compile time with sqlx's `query!` macros against the
//! committed `.sqlx/` offline cache (regenerate with `cargo sqlx prepare` against a
//! live Postgres after changing a query or `migrations/postgres/`). The one
//! exception is the dynamic payload-filter `WHERE` clause, built with
//! [`sqlx::QueryBuilder`] — see the "Filter pushdown" section.
//!
//! # Migrations
//!
//! As with the SQLite backend, applying the schema is an **explicit** step
//! decoupled from constructing the store — [`PgStore::connect`] assumes an
//! already-migrated pool and runs no DDL. This matters *more* for Postgres: a
//! multi-pod deployment against one database would otherwise have every process
//! race the migration advisory lock on boot. Run [`migrate`] / [`migrator`] once
//! as a deliberate step (e.g. a release-phase job), then
//! [`connect`](PgStore::connect). [`PgStore::connect_and_migrate`] bundles the two
//! for single-process / test setups where migrate-on-startup is fine.

use bytes::Bytes;
use sqlx::postgres::{PgPool, PgRow};
use sqlx::types::Json;
use sqlx::{PgConnection, Postgres};
use uuid::Uuid;

use super::sql_common::{InverseResolver, merge, record_err};
use crate::filter::{CompareOp, Filter, Predicate};
use crate::label::Label;
use crate::name::ResourceName;
use crate::object::{Association, Object};
use crate::store::{
    AssociationStore, AssociationStoreReader, EdgeEndpoint, EdgeQuery, ObjectStore,
    ObjectStoreReader, Precondition, StoreExec, StoreTx, Transactional,
};
use crate::{Error, Result};

/// The OpenTelemetry `db.system` value for this backend's operation spans.
const DB_SYSTEM: &str = "postgresql";

/// The crate's embedded Postgres schema [`Migrator`](sqlx::migrate::Migrator).
///
/// This is the primary way to apply the store's Postgres schema. Migrations are a
/// deliberate, explicit step — [`PgStore::connect`] does **not** run them for you
/// (see the module docs for why eager migrate-on-connect is the wrong default for
/// a multi-pod deployment). The migrator is embedded at compile time (via
/// [`sqlx::migrate!`]) and runs against anything implementing [`sqlx::Acquire`]:
///
/// ```no_run
/// # async fn run(pool: sqlx::PgPool) -> Result<(), Box<dyn std::error::Error>> {
/// olai_store::backend::postgres::migrator().run(&pool).await?;
/// # Ok(())
/// # }
/// ```
///
/// `Migrator::run` takes a Postgres advisory migration lock, so concurrent callers
/// serialize safely — but prefer running migrations once as a gated step over
/// racing them from every process on boot.
pub fn migrator() -> sqlx::migrate::Migrator {
    sqlx::migrate!("./migrations/postgres")
}

/// Build a single [`Migrator`](sqlx::migrate::Migrator) over this crate's Postgres
/// schema migrations **plus** the consumer's own, in one ordered ledger.
///
/// The Postgres analogue of [`sqlite::migrator_with`](super::sqlite::migrator_with);
/// see that function for the rationale (one `_sqlx_migrations` ledger, ordinary
/// version ordering, no `ignore_missing`). Number your migrations above this
/// crate's (which occupy the low range).
///
/// ```no_run
/// # async fn run(pool: sqlx::PgPool) -> Result<(), Box<dyn std::error::Error>> {
/// let local = sqlx::migrate!("./migrations");
/// olai_store::pg_migrator_with(local.migrations.iter().cloned())
///     .run(&pool)
///     .await?;
/// # Ok(())
/// # }
/// ```
pub fn migrator_with(
    extra: impl IntoIterator<Item = sqlx::migrate::Migration>,
) -> sqlx::migrate::Migrator {
    merge(migrator(), extra)
}

/// Apply the crate's Postgres schema migrations to `pool`.
///
/// A convenience wrapper over [`migrator`]. Migrations are idempotent and
/// versioned: running them against an already-current database is a no-op. Call it
/// as a deliberate step (a deploy/release job or test setup), then hand the
/// migrated pool to [`PgStore::connect`]. For single-process setups where
/// migrate-on-startup is fine, [`PgStore::connect_and_migrate`] bundles the two.
///
/// # Errors
///
/// Returns a backend error if a migration fails to apply.
pub async fn migrate(pool: &PgPool) -> Result<()> {
    migrator()
        .run(pool)
        .await
        .map_err(|e| Error::generic(e.to_string()))
}

/// A Postgres-backed [`ObjectStore`] + [`AssociationStore`] + [`Transactional`].
#[cfg_attr(docsrs, doc(cfg(feature = "postgres")))]
#[derive(Clone)]
pub struct PgStore<L: Label> {
    pool: PgPool,
    inverse: InverseResolver,
    _label: std::marker::PhantomData<L>,
}

impl<L: Label> PgStore<L> {
    /// Wrap an existing, **already-migrated** pool.
    ///
    /// This does **not** apply migrations — it assumes the schema is already in
    /// place. Coupling DDL to store construction is the wrong default for a
    /// multi-pod deployment: every process would race the migration advisory lock
    /// on boot, and operators lose the ability to run migrations as a deliberate,
    /// gated deploy step. Apply the schema once, explicitly, via [`migrate`] /
    /// [`migrator`] (typically a release-phase job), then call this.
    ///
    /// For single-process or test setups where migrate-on-startup is fine, use
    /// [`connect_and_migrate`](Self::connect_and_migrate).
    pub fn connect(pool: PgPool) -> Self {
        Self {
            pool,
            inverse: std::sync::Arc::new(|_| None),
            _label: std::marker::PhantomData,
        }
    }

    /// Apply the schema migrations to `pool`, then wrap it.
    ///
    /// A convenience for single-process / test setups where migrate-on-startup is
    /// acceptable. **Do not** use this as the default in a multi-pod deployment —
    /// migrate explicitly and use [`connect`](Self::connect) instead.
    ///
    /// # Errors
    ///
    /// Returns a backend error if a migration fails.
    pub async fn connect_and_migrate(pool: PgPool) -> Result<Self> {
        migrate(&pool).await?;
        Ok(Self::connect(pool))
    }

    /// Maintain inverse edges via `resolver`. Chainable onto either constructor.
    #[must_use]
    pub fn with_inverse(
        mut self,
        resolver: impl Fn(&str) -> Option<String> + Send + Sync + 'static,
    ) -> Self {
        self.inverse = std::sync::Arc::new(resolver);
        self
    }
}

// --- Row → domain decoding. Postgres hands back native types (uuid, timestamptz,
//     jsonb, bigint), so the assembler takes them directly rather than parsing
//     strings the way the SQLite backend does. ---

#[allow(clippy::too_many_arguments)]
fn build_object<L: Label>(
    id: Uuid,
    label: String,
    name: String,
    properties: Option<serde_json::Value>,
    version: i64,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<Object<L>> {
    Ok(Object {
        id,
        label: L::from_str(&label).map_err(|_| Error::generic("unknown label in row"))?,
        name: name.parse()?,
        properties,
        version: version as u64,
        created_at,
        updated_at,
    })
}

// --- Operations over a single `PgConnection`, so the same code runs on a pooled
//     connection (auto-commit) or inside a transaction. ---

async fn op_get<L: Label>(conn: &mut PgConnection, id: &Uuid) -> Result<Object<L>> {
    let row = sqlx::query!(
        r#"SELECT id, label, name, properties as "properties: Json<serde_json::Value>",
                  version, created_at, updated_at
           FROM objects WHERE id = $1"#,
        id
    )
    .fetch_optional(conn)
    .await?
    .ok_or(Error::NotFound)?;
    build_object(
        row.id,
        row.label,
        row.name,
        row.properties.map(|j| j.0),
        row.version,
        row.created_at,
        row.updated_at,
    )
}

async fn op_get_by_name<L: Label>(
    conn: &mut PgConnection,
    label: L,
    name: &ResourceName,
) -> Result<Object<L>> {
    let label_s = label.as_str().to_string();
    let name_s = name.to_string();
    let row = sqlx::query!(
        r#"SELECT id, label, name, properties as "properties: Json<serde_json::Value>",
                  version, created_at, updated_at
           FROM objects WHERE label = $1 AND name = $2"#,
        label_s,
        name_s
    )
    .fetch_optional(conn)
    .await?
    .ok_or(Error::NotFound)?;
    build_object(
        row.id,
        row.label,
        row.name,
        row.properties.map(|j| j.0),
        row.version,
        row.created_at,
        row.updated_at,
    )
}

/// Map the object rows from any of the `op_list_objects` `query!` arms (each a distinct
/// anonymous row type with the same columns) into `Object`s. A macro rather than a function
/// because the row type differs per `query!` invocation and cannot be named. (Mirrors the
/// SQLite backend's helper of the same name.)
macro_rules! rows_to_objects {
    ($rows:expr) => {
        $rows
            .into_iter()
            .map(|r| {
                build_object(
                    r.id,
                    r.label,
                    r.name,
                    r.properties.map(|j| j.0),
                    r.version,
                    r.created_at,
                    r.updated_at,
                )
            })
            .collect::<Result<Vec<_>>>()
    };
}

async fn op_list_objects<L: Label>(
    conn: &mut PgConnection,
    label: L,
    namespace: Option<&ResourceName>,
    max_results: Option<usize>,
    page_token: Option<String>,
) -> Result<(Vec<Object<L>>, Option<String>)> {
    let q = crate::store::object_fingerprint(label, namespace, None);
    let cursor = crate::store::decode_cursor(page_token, q)?;
    let limit = max_results.unwrap_or(usize::MAX);
    let label_s = label.as_str().to_string();

    // The namespace filter is a prefix over the escaped `ResourceName` string, which does not
    // translate to a `LIKE` the DB can page on. So when a namespace is set, fetch all label rows
    // in id order and keyset-paginate the filtered result in-process; without a namespace, bound
    // the scan in SQL with `id > :cursor` and over-fetch one. (Mirrors the SQLite backend; note
    // the cursor binds as a native `uuid`, not a string, to match the `uuid` column.)
    if let Some(ns) = namespace {
        let rows = sqlx::query!(
            r#"SELECT id, label, name, properties as "properties: Json<serde_json::Value>",
                      version, created_at, updated_at
               FROM objects WHERE label = $1 ORDER BY id"#,
            label_s
        )
        .fetch_all(conn)
        .await?;
        let mut objects: Vec<Object<L>> = rows_to_objects!(rows)?;
        objects.retain(|o| o.name.prefix_matches(ns));
        if let Some(k) = cursor {
            objects.retain(|o| o.id > k);
        }
        return Ok(crate::store::paginate_keyset(
            objects,
            max_results,
            |o| o.id,
            q,
        ));
    }

    let fetch = limit.saturating_add(1).min(i64::MAX as usize) as i64;
    let objects = match cursor {
        Some(k) => {
            let rows = sqlx::query!(
                r#"SELECT id, label, name, properties as "properties: Json<serde_json::Value>",
                          version, created_at, updated_at
                   FROM objects WHERE label = $1 AND id > $2 ORDER BY id LIMIT $3"#,
                label_s,
                k,
                fetch
            )
            .fetch_all(conn)
            .await?;
            rows_to_objects!(rows)?
        }
        None => {
            let rows = sqlx::query!(
                r#"SELECT id, label, name, properties as "properties: Json<serde_json::Value>",
                          version, created_at, updated_at
                   FROM objects WHERE label = $1 ORDER BY id LIMIT $2"#,
                label_s,
                fetch
            )
            .fetch_all(conn)
            .await?;
            rows_to_objects!(rows)?
        }
    };
    Ok(crate::store::paginate_keyset(
        objects,
        max_results,
        |o| o.id,
        q,
    ))
}

// --- Filter pushdown ------------------------------------------------------------------------
//
// `search` translates the subset of the `Filter` AST that maps *faithfully* to Postgres into a
// `WHERE` clause over `jsonb` operators (`->` / `->>` / `jsonb_typeof`), so the database does the
// filtering and real `LIMIT`/`OFFSET` pagination. Anything outside that subset falls back to the
// trait's Rust-side default (drain the full listing, filter with `Filter::matches`).
//
// The translated query is built with [`sqlx::QueryBuilder`], so it is **not** compile-time
// checked and has **no** `.sqlx` offline-cache entry — the accepted tradeoff for a dynamic
// predicate. All values and JSON keys are `push_bind`'d; only static column identifiers and
// operators are written into the SQL text, so no user input is ever interpolated.

/// Which predicates translate faithfully to Postgres. The reference evaluator
/// ([`Filter::matches`]) is the source of truth; a predicate is only pushed when its SQL form
/// provably agrees with it.
///
/// Pushable: [`Exists`](Predicate::Exists); and [`Eq`](CompareOp::Eq) / ordered comparisons on a
/// scalar (string / number / bool) value, combined with `And`/`Or`/`Not`. Everything else —
/// `Contains`, `Ne`, and comparisons whose query value is `null`, an array, or an object — is
/// left for the Rust fallback.
fn is_pushable(filter: &Filter) -> bool {
    match filter {
        Filter::And(fs) | Filter::Or(fs) => fs.iter().all(is_pushable),
        Filter::Not(f) => is_pushable(f),
        Filter::Predicate(Predicate::Exists { .. }) => true,
        Filter::Predicate(Predicate::Compare { op, value, .. }) => match op {
            // `Ne`/`Contains` are deliberately not pushed.
            CompareOp::Ne | CompareOp::Contains => false,
            // Only scalar comparands translate; null/array/object are left to the fallback.
            CompareOp::Eq | CompareOp::Lt | CompareOp::Le | CompareOp::Gt | CompareOp::Ge => {
                value.is_string() || value.is_number() || value.is_boolean()
            }
        },
    }
}

/// The `jsonb_typeof` values a scalar comparand is allowed to match, mirroring [`crate::filter`]'s
/// `ordering`/`equal`: numbers match `number`, strings match `string`, booleans match `boolean`. A
/// row whose value at the path has any other type (or is absent) must not match — matching the
/// evaluator's "type mismatch ⇒ no match".
fn allowed_type(value: &serde_json::Value) -> &'static str {
    if value.is_number() {
        "number"
    } else if value.is_string() {
        "string"
    } else {
        // booleans (is_pushable already excluded everything else)
        "boolean"
    }
}

/// Emit the SQL comparison operator for a pushable [`CompareOp`].
fn sql_op(op: CompareOp) -> &'static str {
    match op {
        CompareOp::Eq => "=",
        CompareOp::Lt => "<",
        CompareOp::Le => "<=",
        CompareOp::Gt => ">",
        CompareOp::Ge => ">=",
        CompareOp::Ne | CompareOp::Contains => unreachable!("not pushable"),
    }
}

/// Push a `jsonb` path navigation onto `qb` as `(<col> #> $path)`, binding the field path as a
/// Postgres `text[]` so nested keys resolve without any string interpolation. `extract_text`
/// controls the final accessor: `#>>` (returns `text`) vs `#>` (returns `jsonb`).
fn push_path(
    qb: &mut sqlx::QueryBuilder<'_, Postgres>,
    col: &str,
    path: &crate::filter::FieldPath,
    extract_text: bool,
) {
    let segs: Vec<String> = path.segments().iter().map(|s| s.to_string()).collect();
    qb.push("(");
    qb.push(col);
    qb.push(if extract_text { " #>> " } else { " #> " });
    qb.push_bind(segs);
    qb.push(")");
}

/// Recursively emit `filter` into `qb` as a boolean SQL expression that evaluates to a definite
/// TRUE/FALSE (never SQL `NULL`), so it composes correctly under `AND`/`OR`/`NOT`.
///
/// Precondition: `is_pushable(filter)` is `true`.
fn build_where(qb: &mut sqlx::QueryBuilder<'_, Postgres>, filter: &Filter) {
    match filter {
        // Empty And ⇒ true; empty Or ⇒ false; matching the evaluator's identities.
        Filter::And(fs) if fs.is_empty() => {
            qb.push("TRUE");
        }
        Filter::Or(fs) if fs.is_empty() => {
            qb.push("FALSE");
        }
        Filter::And(fs) | Filter::Or(fs) => {
            let sep = if matches!(filter, Filter::And(_)) {
                " AND "
            } else {
                " OR "
            };
            qb.push("(");
            for (i, f) in fs.iter().enumerate() {
                if i > 0 {
                    qb.push(sep);
                }
                build_where(qb, f);
            }
            qb.push(")");
        }
        Filter::Not(f) => {
            // `build_where` yields a definite TRUE/FALSE, so plain NOT can't leak NULL.
            qb.push("(NOT ");
            build_where(qb, f);
            qb.push(")");
        }
        Filter::Predicate(Predicate::Exists { path }) => {
            // Present (including JSON null) ⇔ the path navigates to a non-NULL jsonb; absent ⇔ the
            // `#>` navigation yields SQL NULL. `IS NOT NULL` collapses that to a definite boolean.
            qb.push("(");
            push_path(qb, "properties", path, false);
            qb.push(" IS NOT NULL)");
        }
        Filter::Predicate(Predicate::Compare { path, op, value }) => {
            // Two-valued result: a type guard AND the value comparison, wrapped in COALESCE so an
            // absent path (jsonb_typeof NULL) yields FALSE rather than NULL.
            qb.push("COALESCE((jsonb_typeof(");
            push_path(qb, "properties", path, false);
            qb.push(") = ");
            qb.push_bind(allowed_type(value));
            qb.push(") AND (");
            bind_comparison(qb, path, *op, value);
            qb.push("), FALSE)");
        }
    }
}

/// Emit the scalar value comparison `(<path> <op> <value>)`, extracting the stored value in the
/// same domain the evaluator compares in: numbers as `numeric`, booleans as `boolean`, strings as
/// `text`. The type guard in [`build_where`] has already restricted rows to the matching
/// `jsonb_typeof`, so these casts never see an incompatible value.
fn bind_comparison(
    qb: &mut sqlx::QueryBuilder<'_, Postgres>,
    path: &crate::filter::FieldPath,
    op: CompareOp,
    value: &serde_json::Value,
) {
    match value {
        serde_json::Value::Number(n) => {
            qb.push("(");
            push_path(qb, "properties", path, true);
            qb.push(")::numeric ");
            qb.push(sql_op(op));
            qb.push(" ");
            qb.push_bind(n.as_f64().unwrap_or(f64::NAN));
        }
        serde_json::Value::Bool(b) => {
            qb.push("(");
            push_path(qb, "properties", path, true);
            qb.push(")::boolean ");
            qb.push(sql_op(op));
            qb.push(" ");
            qb.push_bind(*b);
        }
        serde_json::Value::String(s) => {
            // `COLLATE "C"` forces byte-order comparison, matching the reference evaluator
            // (`filter::ordering` compares strings by Rust `str` order = byte order). Without
            // it the extracted text compares under the database's default collation (e.g.
            // `en_US.utf8`), whose linguistic ordering disagrees with the evaluator — so a
            // pushed `Lt`/`Le`/`Gt`/`Ge` would select a different set than `Filter::matches`.
            // Equality is collation-independent here (the default collation is deterministic),
            // but applying it uniformly keeps the operand's comparison semantics unambiguous.
            push_path(qb, "properties", path, true);
            qb.push(" COLLATE \"C\" ");
            qb.push(sql_op(op));
            qb.push(" ");
            qb.push_bind(s.clone());
        }
        // is_pushable guarantees a scalar; other variants never reach here.
        _ => unreachable!("non-scalar comparand is not pushable"),
    }
}

/// Search objects by payload, pushing the filter into SQL when it translates faithfully.
/// Mirrors the SQLite backend's `op_search_objects` structure; only the emitted SQL differs.
async fn op_search_objects<L: Label>(
    conn: &mut PgConnection,
    label: L,
    namespace: Option<&ResourceName>,
    filter: &Filter,
    max_results: Option<usize>,
    page_token: Option<String>,
) -> Result<(Vec<Object<L>>, Option<String>)> {
    let q = crate::store::object_fingerprint(label, namespace, Some(filter));
    let cursor = crate::store::decode_cursor(page_token, q)?;

    if !is_pushable(filter) {
        // Rust fallback: drain the full (namespaced) listing, then filter + keyset-paginate.
        let (all, _) = op_list_objects(conn, label, namespace, None, None).await?;
        let matched: Vec<_> = all
            .into_iter()
            .filter(|o| filter.matches(crate::store::props_or_null(&o.properties)))
            .filter(|o| cursor.is_none_or(|k| o.id > k))
            .collect();
        return Ok(crate::store::paginate_keyset(
            matched,
            max_results,
            |o| o.id,
            q,
        ));
    }

    let limit = max_results.unwrap_or(usize::MAX);
    let label_s = label.as_str().to_string();

    let mut qb = sqlx::QueryBuilder::<Postgres>::new(
        r#"SELECT id, label, name, properties, version, created_at, updated_at
           FROM objects WHERE label = "#,
    );
    qb.push_bind(label_s);
    qb.push(" AND ");
    build_where(&mut qb, filter);
    // Without a namespace, SQL can page directly: bound by the keyset cursor (a native `uuid`)
    // and over-fetch one row to detect "has more". With a namespace, fetch every filter-matching
    // row and page after the Rust prefix filter.
    if namespace.is_none()
        && let Some(k) = cursor
    {
        qb.push(" AND id > ");
        qb.push_bind(k);
    }
    qb.push(" ORDER BY id");
    if namespace.is_none() {
        let fetch = limit.saturating_add(1).min(i64::MAX as usize) as i64;
        qb.push(" LIMIT ");
        qb.push_bind(fetch);
    }

    let rows = qb.build().fetch_all(conn).await?;
    let mut objects = rows
        .into_iter()
        .map(object_from_row)
        .collect::<Result<Vec<_>>>()?;

    if let Some(ns) = namespace {
        objects.retain(|o| o.name.prefix_matches(ns));
        if let Some(k) = cursor {
            objects.retain(|o| o.id > k);
        }
    }
    Ok(crate::store::paginate_keyset(
        objects,
        max_results,
        |o| o.id,
        q,
    ))
}

/// List edges matching an [`EdgeQuery`], most-recent-first, pushing every predicate it can into
/// one `WHERE`. Mirrors the SQLite backend's `op_query_edges`; only the emitted SQL differs.
///
/// Ordering is `id DESC`: edge ids are UUIDv7 (time-ordered), so descending id is most-recent-first.
async fn op_query_edges<L: Label>(
    conn: &mut PgConnection,
    query: EdgeQuery<'_, L>,
) -> Result<(Vec<Association<L>>, Option<String>)> {
    let q = crate::store::edge_fingerprint(&query);
    let cursor = crate::store::decode_cursor(query.page_token.clone(), q)?;

    // Anchor column + the "other" endpoint column that `target_id` restricts.
    let (anchor_col, other_col, anchor_id) = match query.endpoint {
        EdgeEndpoint::From(id) => ("from_id", "to_id", id),
        EdgeEndpoint::Into(id) => ("to_id", "from_id", id),
    };

    // Emit the anchor + label + target restrictions shared by both paths.
    let base = |qb: &mut sqlx::QueryBuilder<'_, Postgres>| {
        qb.push(" WHERE ");
        qb.push(anchor_col);
        qb.push(" = ");
        qb.push_bind(anchor_id);
        qb.push(" AND label = ");
        qb.push_bind(query.label.to_string());
        if let Some(tl) = query.target_label {
            qb.push(" AND to_label = ");
            qb.push_bind(tl.as_str().to_string());
        }
        if let Some(tid) = query.target_id {
            qb.push(" AND ");
            qb.push(other_col);
            qb.push(" = ");
            qb.push_bind(tid);
        }
        // Time window: v7 ids are time-ordered, so a `[since, until)` window on creation time is
        // an id range. `since` inclusive → `id >= since_lo`; `until` exclusive → `id < until_lo`.
        if let Some(since) = query.since {
            qb.push(" AND id >= ");
            qb.push_bind(crate::store::v7_lower_bound(since));
        }
        if let Some(until) = query.until {
            qb.push(" AND id < ");
            qb.push_bind(crate::store::v7_lower_bound(until));
        }
        // Keyset cursor: edges page descending, so "after the cursor" is a strictly smaller id.
        if let Some(k) = cursor {
            qb.push(" AND id < ");
            qb.push_bind(k);
        }
    };

    const SELECT: &str = "SELECT id, from_id, label, to_id, to_label, properties, created_at, updated_at \
         FROM associations";

    // Non-pushable filter: push everything else, fetch the full ordered set, filter in Rust.
    if let Some(filter) = query.filter
        && !is_pushable(filter)
    {
        let mut qb = sqlx::QueryBuilder::<Postgres>::new(SELECT);
        base(&mut qb);
        qb.push(" ORDER BY id DESC");
        let rows = qb.build().fetch_all(conn).await?;
        let matched: Vec<_> = rows
            .into_iter()
            .map(edge_from_row)
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .filter(|a| filter.matches(crate::store::props_or_null(&a.properties)))
            .collect();
        return Ok(crate::store::paginate_keyset(
            matched,
            query.max_results,
            |e| e.id,
            q,
        ));
    }

    let limit = query.max_results.unwrap_or(usize::MAX);
    let fetch = limit.saturating_add(1).min(i64::MAX as usize) as i64;

    let mut qb = sqlx::QueryBuilder::<Postgres>::new(SELECT);
    base(&mut qb);
    if let Some(filter) = query.filter {
        qb.push(" AND ");
        build_where(&mut qb, filter);
    }
    qb.push(" ORDER BY id DESC LIMIT ");
    qb.push_bind(fetch);

    let rows = qb.build().fetch_all(conn).await?;
    let edges = rows
        .into_iter()
        .map(edge_from_row)
        .collect::<Result<Vec<_>>>()?;
    Ok(crate::store::paginate_keyset(
        edges,
        query.max_results,
        |e| e.id,
        q,
    ))
}

/// Count edges matching an anchor + `label` (+ optional `target_label`) via `COUNT(*)`.
async fn op_count_edges<L: Label>(
    conn: &mut PgConnection,
    endpoint: EdgeEndpoint,
    label: &str,
    target_label: Option<L>,
) -> Result<u64> {
    let (anchor_col, anchor_id) = match endpoint {
        EdgeEndpoint::From(id) => ("from_id", id),
        EdgeEndpoint::Into(id) => ("to_id", id),
    };
    let mut qb = sqlx::QueryBuilder::<Postgres>::new("SELECT COUNT(*) FROM associations WHERE ");
    qb.push(anchor_col);
    qb.push(" = ");
    qb.push_bind(anchor_id);
    qb.push(" AND label = ");
    qb.push_bind(label.to_string());
    if let Some(tl) = target_label {
        qb.push(" AND to_label = ");
        qb.push_bind(tl.as_str().to_string());
    }
    let count: i64 = qb.build_query_scalar().fetch_one(conn).await?;
    Ok(count as u64)
}

/// Build an [`Object`] from a dynamically-queried row (the pushdown path can't use the typed
/// `query!` row, so it reads columns by name via [`sqlx::Row`]).
fn object_from_row<L: Label>(row: PgRow) -> Result<Object<L>> {
    use sqlx::Row;
    let properties: Option<Json<serde_json::Value>> = row.try_get("properties")?;
    build_object(
        row.try_get("id")?,
        row.try_get("label")?,
        row.try_get("name")?,
        properties.map(|j| j.0),
        row.try_get("version")?,
        row.try_get("created_at")?,
        row.try_get("updated_at")?,
    )
}

/// Build an [`Association`] from a dynamically-queried row.
fn edge_from_row<L: Label>(row: PgRow) -> Result<Association<L>> {
    use sqlx::Row;
    let properties: Option<Json<serde_json::Value>> = row.try_get("properties")?;
    Ok(Association {
        id: row.try_get("id")?,
        from_id: row.try_get("from_id")?,
        label: row.try_get("label")?,
        to_id: row.try_get("to_id")?,
        to_label: L::from_str(&row.try_get::<String, _>("to_label")?)
            .map_err(|_| Error::generic("unknown label in row"))?,
        properties: properties.map(|j| j.0),
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

async fn op_create<L: Label>(
    conn: &mut PgConnection,
    label: L,
    name: &ResourceName,
    properties: Option<serde_json::Value>,
    id: Option<Uuid>,
    sensitive: Option<Bytes>,
) -> Result<Object<L>> {
    let object = Object {
        // UUIDv7 (time-ordered) so `id` doubles as the chronological keyset pagination key.
        id: id.unwrap_or_else(Uuid::now_v7),
        label,
        name: name.clone(),
        properties,
        version: 0,
        created_at: chrono::Utc::now(),
        updated_at: None,
    };
    let label_s = object.label.as_str().to_string();
    let name_s = object.name.to_string();
    let props = object.properties.clone().map(Json);
    // The sensitive blob is bound in the same INSERT so the row and its sealed value land
    // atomically (NULL when there is nothing sealed).
    let sensitive = sensitive.as_deref();
    sqlx::query!(
        "INSERT INTO objects (id, label, name, properties, sensitive, version, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, 0, $6, NULL)",
        object.id,
        label_s,
        name_s,
        props as Option<Json<serde_json::Value>>,
        sensitive,
        object.created_at,
    )
    .execute(conn)
    .await?;
    Ok(object)
}

async fn op_get_sensitive(conn: &mut PgConnection, id: &Uuid) -> Result<Option<Bytes>> {
    // A missing object and an object with no blob both yield `Ok(None)` — matching the trait
    // contract ("treat `Ok(None)` as no blob regardless of whether the object exists").
    let row = sqlx::query!(r#"SELECT sensitive FROM objects WHERE id = $1"#, id)
        .fetch_optional(conn)
        .await?;
    Ok(row.and_then(|r| r.sensitive).map(Bytes::from))
}

/// Replace only the `sensitive` column, leaving properties and version untouched.
async fn op_set_sensitive(conn: &mut PgConnection, id: &Uuid, blob: &[u8]) -> Result<()> {
    let affected = sqlx::query!("UPDATE objects SET sensitive = $1 WHERE id = $2", blob, id)
        .execute(conn)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(Error::NotFound);
    }
    Ok(())
}

/// A zero-row conditional write means either the row is gone (`NotFound`) or its version moved
/// (`Conflict`). Re-read to disambiguate.
async fn classify_miss<L: Label>(conn: &mut PgConnection, id: &Uuid) -> Error {
    match op_get::<L>(conn, id).await {
        Ok(_) => {
            tracing::debug!(id = %id, "CAS precondition conflict (version moved)");
            Error::Conflict
        }
        Err(Error::NotFound) => Error::NotFound,
        Err(e) => e,
    }
}

async fn op_update<L: Label>(
    conn: &mut PgConnection,
    id: &Uuid,
    properties: Option<serde_json::Value>,
    precondition: Precondition,
    sensitive: Option<Bytes>,
) -> Result<Object<L>> {
    let props = properties.map(Json);
    let now = chrono::Utc::now();
    let blob = sensitive.as_deref();

    // A `None` blob leaves the stored `sensitive` column untouched; a `Some` blob replaces it in
    // the same statement so properties and the sealed value update atomically. The CAS guard is a
    // real `WHERE … AND version = $n`, and `RETURNING` hands back the updated row so no re-read is
    // needed on success.
    let updated = match (precondition, blob) {
        (Precondition::Any, None) => sqlx::query!(
            r#"UPDATE objects SET properties = $1, version = version + 1, updated_at = $2
               WHERE id = $3
               RETURNING id, label, name, properties as "properties: Json<serde_json::Value>",
                         version, created_at, updated_at"#,
            props as Option<Json<serde_json::Value>>,
            now,
            id
        )
        .fetch_optional(&mut *conn)
        .await?
        .map(|r| build_object(r.id, r.label, r.name, r.properties.map(|j| j.0), r.version, r.created_at, r.updated_at)),
        (Precondition::Any, Some(blob)) => sqlx::query!(
            r#"UPDATE objects SET properties = $1, sensitive = $2, version = version + 1, updated_at = $3
               WHERE id = $4
               RETURNING id, label, name, properties as "properties: Json<serde_json::Value>",
                         version, created_at, updated_at"#,
            props as Option<Json<serde_json::Value>>,
            blob,
            now,
            id
        )
        .fetch_optional(&mut *conn)
        .await?
        .map(|r| build_object(r.id, r.label, r.name, r.properties.map(|j| j.0), r.version, r.created_at, r.updated_at)),
        (Precondition::Version(v), None) => sqlx::query!(
            r#"UPDATE objects SET properties = $1, version = version + 1, updated_at = $2
               WHERE id = $3 AND version = $4
               RETURNING id, label, name, properties as "properties: Json<serde_json::Value>",
                         version, created_at, updated_at"#,
            props as Option<Json<serde_json::Value>>,
            now,
            id,
            v as i64
        )
        .fetch_optional(&mut *conn)
        .await?
        .map(|r| build_object(r.id, r.label, r.name, r.properties.map(|j| j.0), r.version, r.created_at, r.updated_at)),
        (Precondition::Version(v), Some(blob)) => sqlx::query!(
            r#"UPDATE objects SET properties = $1, sensitive = $2, version = version + 1, updated_at = $3
               WHERE id = $4 AND version = $5
               RETURNING id, label, name, properties as "properties: Json<serde_json::Value>",
                         version, created_at, updated_at"#,
            props as Option<Json<serde_json::Value>>,
            blob,
            now,
            id,
            v as i64
        )
        .fetch_optional(&mut *conn)
        .await?
        .map(|r| build_object(r.id, r.label, r.name, r.properties.map(|j| j.0), r.version, r.created_at, r.updated_at)),
    };
    match updated {
        Some(obj) => obj,
        None => Err(classify_miss::<L>(conn, id).await),
    }
}

async fn op_rename<L: Label>(
    conn: &mut PgConnection,
    id: &Uuid,
    new_name: &ResourceName,
    precondition: Precondition,
) -> Result<Object<L>> {
    let name_s = new_name.to_string();
    let now = chrono::Utc::now();

    let updated = match precondition {
        Precondition::Any => sqlx::query!(
            r#"UPDATE objects SET name = $1, version = version + 1, updated_at = $2
               WHERE id = $3
               RETURNING id, label, name, properties as "properties: Json<serde_json::Value>",
                         version, created_at, updated_at"#,
            name_s,
            now,
            id
        )
        .fetch_optional(&mut *conn)
        .await?
        .map(|r| {
            build_object(
                r.id,
                r.label,
                r.name,
                r.properties.map(|j| j.0),
                r.version,
                r.created_at,
                r.updated_at,
            )
        }),
        Precondition::Version(v) => sqlx::query!(
            r#"UPDATE objects SET name = $1, version = version + 1, updated_at = $2
               WHERE id = $3 AND version = $4
               RETURNING id, label, name, properties as "properties: Json<serde_json::Value>",
                         version, created_at, updated_at"#,
            name_s,
            now,
            id,
            v as i64
        )
        .fetch_optional(&mut *conn)
        .await?
        .map(|r| {
            build_object(
                r.id,
                r.label,
                r.name,
                r.properties.map(|j| j.0),
                r.version,
                r.created_at,
                r.updated_at,
            )
        }),
    };
    match updated {
        Some(obj) => obj,
        None => Err(classify_miss::<L>(conn, id).await),
    }
}

async fn op_delete(conn: &mut PgConnection, id: &Uuid) -> Result<()> {
    // Cascade edges (either direction), then the object.
    sqlx::query!(
        "DELETE FROM associations WHERE from_id = $1 OR to_id = $1",
        id
    )
    .execute(&mut *conn)
    .await?;
    let affected = sqlx::query!("DELETE FROM objects WHERE id = $1", id)
        .execute(&mut *conn)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(Error::NotFound);
    }
    Ok(())
}

async fn op_add_edge<L: Label>(
    conn: &mut PgConnection,
    from_id: Uuid,
    to_id: Uuid,
    label: &str,
    properties: Option<serde_json::Value>,
    inverse: &InverseResolver,
) -> Result<()> {
    let from: Object<L> = op_get(&mut *conn, &from_id).await?;
    let to: Object<L> = op_get(&mut *conn, &to_id).await?;
    insert_edge(
        &mut *conn,
        from_id,
        to_id,
        label,
        to.label,
        properties.clone(),
    )
    .await?;
    if let Some(inv) = inverse(label) {
        insert_edge(&mut *conn, to_id, from_id, &inv, from.label, properties).await?;
    }
    Ok(())
}

async fn insert_edge<L: Label>(
    conn: &mut PgConnection,
    from_id: Uuid,
    to_id: Uuid,
    label: &str,
    to_label: L,
    properties: Option<serde_json::Value>,
) -> Result<()> {
    // v7 (time-ordered) so `ORDER BY id DESC` in `op_query_edges` is most-recent-first.
    let id = Uuid::now_v7();
    let to_label_s = to_label.as_str().to_string();
    let props = properties.map(Json);
    let now = chrono::Utc::now();
    sqlx::query!(
        "INSERT INTO associations \
         (id, from_id, label, to_id, to_label, properties, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, NULL)",
        id,
        from_id,
        label,
        to_id,
        to_label_s,
        props as Option<Json<serde_json::Value>>,
        now
    )
    .execute(conn)
    .await?;
    Ok(())
}

async fn op_remove_edge(
    conn: &mut PgConnection,
    from_id: Uuid,
    to_id: Uuid,
    label: &str,
    inverse: &InverseResolver,
) -> Result<()> {
    let affected = sqlx::query!(
        "DELETE FROM associations WHERE from_id = $1 AND to_id = $2 AND label = $3",
        from_id,
        to_id,
        label
    )
    .execute(&mut *conn)
    .await?
    .rows_affected();
    if affected == 0 {
        return Err(Error::NotFound);
    }
    if let Some(inv) = inverse(label) {
        sqlx::query!(
            "DELETE FROM associations WHERE from_id = $1 AND to_id = $2 AND label = $3",
            to_id,
            from_id,
            inv
        )
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

// --- Top-level store: acquire a connection (or open a txn for multi-statement ops) and delegate
//     to the shared `op_*` operations. ---

#[async_trait::async_trait]
impl<L: Label> ObjectStoreReader<L> for PgStore<L> {
    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.get", otel.kind = "client", db.system = DB_SYSTEM,
            db.operation.name = "get", id = %id,
            otel.status_code = tracing::field::Empty, error.type = tracing::field::Empty,
        )
    )]
    async fn get(&self, id: &Uuid) -> Result<Object<L>> {
        let mut conn = self.pool.acquire().await?;
        let out = op_get(&mut conn, id).await;
        record_err(&out);
        out
    }

    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.get_by_name", otel.kind = "client", db.system = DB_SYSTEM,
            db.operation.name = "get_by_name", db.collection.name = label.as_str(), name = %name,
            otel.status_code = tracing::field::Empty, error.type = tracing::field::Empty,
        )
    )]
    async fn get_by_name(&self, label: L, name: &ResourceName) -> Result<Object<L>> {
        let mut conn = self.pool.acquire().await?;
        let out = op_get_by_name(&mut conn, label, name).await;
        record_err(&out);
        out
    }

    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.list", otel.kind = "client", db.system = DB_SYSTEM,
            db.operation.name = "list", db.collection.name = label.as_str(), max_results = ?max_results,
            otel.status_code = tracing::field::Empty, error.type = tracing::field::Empty,
        )
    )]
    async fn list(
        &self,
        label: L,
        namespace: Option<&ResourceName>,
        max_results: Option<usize>,
        page_token: Option<String>,
    ) -> Result<(Vec<Object<L>>, Option<String>)> {
        let mut conn = self.pool.acquire().await?;
        let out = op_list_objects(&mut conn, label, namespace, max_results, page_token).await;
        record_err(&out);
        out
    }

    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.search", otel.kind = "client", db.system = DB_SYSTEM,
            db.operation.name = "search", db.collection.name = label.as_str(), max_results = ?max_results,
            otel.status_code = tracing::field::Empty, error.type = tracing::field::Empty,
        )
    )]
    async fn search(
        &self,
        label: L,
        namespace: Option<&ResourceName>,
        filter: &Filter,
        max_results: Option<usize>,
        page_token: Option<String>,
    ) -> Result<(Vec<Object<L>>, Option<String>)> {
        let mut conn = self.pool.acquire().await?;
        let out =
            op_search_objects(&mut conn, label, namespace, filter, max_results, page_token).await;
        record_err(&out);
        out
    }

    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.get_sensitive", otel.kind = "client", db.system = DB_SYSTEM,
            db.operation.name = "get_sensitive", id = %id,
            otel.status_code = tracing::field::Empty, error.type = tracing::field::Empty,
        )
    )]
    async fn get_sensitive(&self, id: &Uuid) -> Result<Option<Bytes>> {
        let mut conn = self.pool.acquire().await?;
        let out = op_get_sensitive(&mut conn, id).await;
        record_err(&out);
        out
    }
}

#[async_trait::async_trait]
impl<L: Label> ObjectStore<L> for PgStore<L> {
    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.create", otel.kind = "client", db.system = DB_SYSTEM,
            db.operation.name = "create", db.collection.name = label.as_str(), name = %name,
            otel.status_code = tracing::field::Empty, error.type = tracing::field::Empty,
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
        let mut conn = self.pool.acquire().await?;
        let out = op_create(&mut conn, label, name, properties, id, sensitive).await;
        record_err(&out);
        out
    }

    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.update", otel.kind = "client", db.system = DB_SYSTEM,
            db.operation.name = "update", id = %id, precondition = ?precondition,
            otel.status_code = tracing::field::Empty, error.type = tracing::field::Empty,
        )
    )]
    async fn update(
        &self,
        id: &Uuid,
        properties: Option<serde_json::Value>,
        precondition: Precondition,
        sensitive: Option<Bytes>,
    ) -> Result<Object<L>> {
        let out = async {
            let mut tx = self.pool.begin().await?;
            let out = op_update(&mut tx, id, properties, precondition, sensitive).await?;
            tx.commit().await?;
            Ok(out)
        }
        .await;
        record_err(&out);
        out
    }

    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.rename", otel.kind = "client", db.system = DB_SYSTEM,
            db.operation.name = "rename", id = %id, name = %new_name, precondition = ?precondition,
            otel.status_code = tracing::field::Empty, error.type = tracing::field::Empty,
        )
    )]
    async fn rename(
        &self,
        id: &Uuid,
        new_name: &ResourceName,
        precondition: Precondition,
    ) -> Result<Object<L>> {
        let out = async {
            let mut tx = self.pool.begin().await?;
            let out = op_rename(&mut tx, id, new_name, precondition).await?;
            tx.commit().await?;
            Ok(out)
        }
        .await;
        record_err(&out);
        out
    }

    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.delete", otel.kind = "client", db.system = DB_SYSTEM,
            db.operation.name = "delete", id = %id,
            otel.status_code = tracing::field::Empty, error.type = tracing::field::Empty,
        )
    )]
    async fn delete(&self, id: &Uuid) -> Result<()> {
        let out = async {
            let mut tx = self.pool.begin().await?;
            op_delete(&mut tx, id).await?;
            tx.commit().await?;
            Ok(())
        }
        .await;
        record_err(&out);
        out
    }

    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.set_sensitive", otel.kind = "client", db.system = DB_SYSTEM,
            db.operation.name = "set_sensitive", id = %id,
            otel.status_code = tracing::field::Empty, error.type = tracing::field::Empty,
        )
    )]
    async fn set_sensitive(&self, id: &Uuid, sensitive: Bytes) -> Result<()> {
        let mut conn = self.pool.acquire().await?;
        let out = op_set_sensitive(&mut conn, id, &sensitive).await;
        record_err(&out);
        out
    }
}

#[async_trait::async_trait]
impl<L: Label> AssociationStoreReader<L> for PgStore<L> {
    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.query_edges", otel.kind = "client", db.system = DB_SYSTEM,
            db.operation.name = "query_edges", label = %query.label,
            otel.status_code = tracing::field::Empty, error.type = tracing::field::Empty,
        )
    )]
    async fn query_edges(
        &self,
        query: EdgeQuery<'_, L>,
    ) -> Result<(Vec<Association<L>>, Option<String>)> {
        let mut conn = self.pool.acquire().await?;
        let out = op_query_edges(&mut conn, query).await;
        record_err(&out);
        out
    }

    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.count_edges", otel.kind = "client", db.system = DB_SYSTEM,
            db.operation.name = "count_edges", label = %label,
            otel.status_code = tracing::field::Empty, error.type = tracing::field::Empty,
        )
    )]
    async fn count_edges(
        &self,
        endpoint: EdgeEndpoint,
        label: &str,
        target_label: Option<L>,
    ) -> Result<u64> {
        let mut conn = self.pool.acquire().await?;
        let out = op_count_edges(&mut conn, endpoint, label, target_label).await;
        record_err(&out);
        out
    }
}

#[async_trait::async_trait]
impl<L: Label> AssociationStore<L> for PgStore<L> {
    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.add_edge", otel.kind = "client", db.system = DB_SYSTEM,
            db.operation.name = "add_edge", label = %label, from_id = %from_id, to_id = %to_id,
            otel.status_code = tracing::field::Empty, error.type = tracing::field::Empty,
        )
    )]
    async fn add(
        &self,
        from_id: Uuid,
        to_id: Uuid,
        label: &str,
        properties: Option<serde_json::Value>,
    ) -> Result<()> {
        let out = async {
            let mut tx = self.pool.begin().await?;
            op_add_edge::<L>(&mut tx, from_id, to_id, label, properties, &self.inverse).await?;
            tx.commit().await?;
            Ok(())
        }
        .await;
        record_err(&out);
        out
    }

    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.remove_edge", otel.kind = "client", db.system = DB_SYSTEM,
            db.operation.name = "remove_edge", label = %label, from_id = %from_id, to_id = %to_id,
            otel.status_code = tracing::field::Empty, error.type = tracing::field::Empty,
        )
    )]
    async fn remove(&self, from_id: Uuid, to_id: Uuid, label: &str) -> Result<()> {
        let out = async {
            let mut tx = self.pool.begin().await?;
            op_remove_edge(&mut tx, from_id, to_id, label, &self.inverse).await?;
            tx.commit().await?;
            Ok(())
        }
        .await;
        record_err(&out);
        out
    }
}

// --- Transactions ---

/// An open Postgres transaction handle. Buffers writes until [`commit`](StoreTx::commit);
/// dropping rolls back.
pub struct PgTx<L: Label> {
    tx: tokio::sync::Mutex<sqlx::Transaction<'static, Postgres>>,
    inverse: InverseResolver,
    _label: std::marker::PhantomData<L>,
}

#[async_trait::async_trait]
impl<L: Label> ObjectStoreReader<L> for PgTx<L> {
    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.get", otel.kind = "client", db.system = DB_SYSTEM,
            db.operation.name = "get", id = %id,
            otel.status_code = tracing::field::Empty, error.type = tracing::field::Empty,
        )
    )]
    async fn get(&self, id: &Uuid) -> Result<Object<L>> {
        let mut tx = self.tx.lock().await;
        let out = op_get(&mut tx, id).await;
        record_err(&out);
        out
    }

    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.get_by_name", otel.kind = "client", db.system = DB_SYSTEM,
            db.operation.name = "get_by_name", db.collection.name = label.as_str(), name = %name,
            otel.status_code = tracing::field::Empty, error.type = tracing::field::Empty,
        )
    )]
    async fn get_by_name(&self, label: L, name: &ResourceName) -> Result<Object<L>> {
        let mut tx = self.tx.lock().await;
        let out = op_get_by_name(&mut tx, label, name).await;
        record_err(&out);
        out
    }

    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.list", otel.kind = "client", db.system = DB_SYSTEM,
            db.operation.name = "list", db.collection.name = label.as_str(), max_results = ?max_results,
            otel.status_code = tracing::field::Empty, error.type = tracing::field::Empty,
        )
    )]
    async fn list(
        &self,
        label: L,
        namespace: Option<&ResourceName>,
        max_results: Option<usize>,
        page_token: Option<String>,
    ) -> Result<(Vec<Object<L>>, Option<String>)> {
        let mut tx = self.tx.lock().await;
        let out = op_list_objects(&mut tx, label, namespace, max_results, page_token).await;
        record_err(&out);
        out
    }

    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.search", otel.kind = "client", db.system = DB_SYSTEM,
            db.operation.name = "search", db.collection.name = label.as_str(), max_results = ?max_results,
            otel.status_code = tracing::field::Empty, error.type = tracing::field::Empty,
        )
    )]
    async fn search(
        &self,
        label: L,
        namespace: Option<&ResourceName>,
        filter: &Filter,
        max_results: Option<usize>,
        page_token: Option<String>,
    ) -> Result<(Vec<Object<L>>, Option<String>)> {
        let mut tx = self.tx.lock().await;
        let out =
            op_search_objects(&mut tx, label, namespace, filter, max_results, page_token).await;
        record_err(&out);
        out
    }

    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.get_sensitive", otel.kind = "client", db.system = DB_SYSTEM,
            db.operation.name = "get_sensitive", id = %id,
            otel.status_code = tracing::field::Empty, error.type = tracing::field::Empty,
        )
    )]
    async fn get_sensitive(&self, id: &Uuid) -> Result<Option<Bytes>> {
        let mut tx = self.tx.lock().await;
        let out = op_get_sensitive(&mut tx, id).await;
        record_err(&out);
        out
    }
}

#[async_trait::async_trait]
impl<L: Label> ObjectStore<L> for PgTx<L> {
    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.create", otel.kind = "client", db.system = DB_SYSTEM,
            db.operation.name = "create", db.collection.name = label.as_str(), name = %name,
            otel.status_code = tracing::field::Empty, error.type = tracing::field::Empty,
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
        let mut tx = self.tx.lock().await;
        let out = op_create(&mut tx, label, name, properties, id, sensitive).await;
        record_err(&out);
        out
    }

    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.update", otel.kind = "client", db.system = DB_SYSTEM,
            db.operation.name = "update", id = %id, precondition = ?precondition,
            otel.status_code = tracing::field::Empty, error.type = tracing::field::Empty,
        )
    )]
    async fn update(
        &self,
        id: &Uuid,
        properties: Option<serde_json::Value>,
        precondition: Precondition,
        sensitive: Option<Bytes>,
    ) -> Result<Object<L>> {
        let mut tx = self.tx.lock().await;
        let out = op_update(&mut tx, id, properties, precondition, sensitive).await;
        record_err(&out);
        out
    }

    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.rename", otel.kind = "client", db.system = DB_SYSTEM,
            db.operation.name = "rename", id = %id, name = %new_name, precondition = ?precondition,
            otel.status_code = tracing::field::Empty, error.type = tracing::field::Empty,
        )
    )]
    async fn rename(
        &self,
        id: &Uuid,
        new_name: &ResourceName,
        precondition: Precondition,
    ) -> Result<Object<L>> {
        let mut tx = self.tx.lock().await;
        let out = op_rename(&mut tx, id, new_name, precondition).await;
        record_err(&out);
        out
    }

    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.delete", otel.kind = "client", db.system = DB_SYSTEM,
            db.operation.name = "delete", id = %id,
            otel.status_code = tracing::field::Empty, error.type = tracing::field::Empty,
        )
    )]
    async fn delete(&self, id: &Uuid) -> Result<()> {
        let mut tx = self.tx.lock().await;
        let out = op_delete(&mut tx, id).await;
        record_err(&out);
        out
    }

    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.set_sensitive", otel.kind = "client", db.system = DB_SYSTEM,
            db.operation.name = "set_sensitive", id = %id,
            otel.status_code = tracing::field::Empty, error.type = tracing::field::Empty,
        )
    )]
    async fn set_sensitive(&self, id: &Uuid, sensitive: Bytes) -> Result<()> {
        let mut tx = self.tx.lock().await;
        let out = op_set_sensitive(&mut tx, id, &sensitive).await;
        record_err(&out);
        out
    }
}

#[async_trait::async_trait]
impl<L: Label> AssociationStoreReader<L> for PgTx<L> {
    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.query_edges", otel.kind = "client", db.system = DB_SYSTEM,
            db.operation.name = "query_edges", label = %query.label,
            otel.status_code = tracing::field::Empty, error.type = tracing::field::Empty,
        )
    )]
    async fn query_edges(
        &self,
        query: EdgeQuery<'_, L>,
    ) -> Result<(Vec<Association<L>>, Option<String>)> {
        let mut tx = self.tx.lock().await;
        let out = op_query_edges(&mut tx, query).await;
        record_err(&out);
        out
    }

    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.count_edges", otel.kind = "client", db.system = DB_SYSTEM,
            db.operation.name = "count_edges", label = %label,
            otel.status_code = tracing::field::Empty, error.type = tracing::field::Empty,
        )
    )]
    async fn count_edges(
        &self,
        endpoint: EdgeEndpoint,
        label: &str,
        target_label: Option<L>,
    ) -> Result<u64> {
        let mut tx = self.tx.lock().await;
        let out = op_count_edges(&mut tx, endpoint, label, target_label).await;
        record_err(&out);
        out
    }
}

#[async_trait::async_trait]
impl<L: Label> AssociationStore<L> for PgTx<L> {
    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.add_edge", otel.kind = "client", db.system = DB_SYSTEM,
            db.operation.name = "add_edge", label = %label, from_id = %from_id, to_id = %to_id,
            otel.status_code = tracing::field::Empty, error.type = tracing::field::Empty,
        )
    )]
    async fn add(
        &self,
        from_id: Uuid,
        to_id: Uuid,
        label: &str,
        properties: Option<serde_json::Value>,
    ) -> Result<()> {
        let inverse = self.inverse.clone();
        let mut tx = self.tx.lock().await;
        let out = op_add_edge::<L>(&mut tx, from_id, to_id, label, properties, &inverse).await;
        record_err(&out);
        out
    }

    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.remove_edge", otel.kind = "client", db.system = DB_SYSTEM,
            db.operation.name = "remove_edge", label = %label, from_id = %from_id, to_id = %to_id,
            otel.status_code = tracing::field::Empty, error.type = tracing::field::Empty,
        )
    )]
    async fn remove(&self, from_id: Uuid, to_id: Uuid, label: &str) -> Result<()> {
        let inverse = self.inverse.clone();
        let mut tx = self.tx.lock().await;
        let out = op_remove_edge(&mut tx, from_id, to_id, label, &inverse).await;
        record_err(&out);
        out
    }
}

#[async_trait::async_trait]
impl<L: Label> StoreTx<L> for PgTx<L> {
    #[tracing::instrument(
        skip_all,
        fields(otel.kind = "client", db.system = DB_SYSTEM, db.operation.name = "commit")
    )]
    async fn commit(self: Box<Self>) -> Result<()> {
        self.tx.into_inner().commit().await?;
        Ok(())
    }

    #[tracing::instrument(
        skip_all,
        fields(otel.kind = "client", db.system = DB_SYSTEM, db.operation.name = "rollback")
    )]
    async fn rollback(self: Box<Self>) -> Result<()> {
        self.tx.into_inner().rollback().await?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl<L: Label> Transactional<L> for PgStore<L> {
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
        let tx = self.pool.begin().await?;
        let handle = PgTx::<L> {
            tx: tokio::sync::Mutex::new(tx),
            inverse: self.inverse.clone(),
            _label: std::marker::PhantomData,
        };
        match f(&handle).await {
            Ok(value) => {
                handle.tx.into_inner().commit().await?;
                Ok(value)
            }
            Err(e) => {
                let span = tracing::Span::current();
                span.record("otel.status_code", "ERROR");
                span.record("error.type", e.kind_str());
                if let Err(rb) = handle.tx.into_inner().rollback().await {
                    tracing::warn!(error = %rb, "transaction rollback failed after operation error");
                }
                Err(e)
            }
        }
    }

    #[tracing::instrument(
        skip_all,
        fields(otel.kind = "client", db.system = DB_SYSTEM, db.operation.name = "begin")
    )]
    async fn begin(&self) -> Result<Box<dyn StoreTx<L>>> {
        let tx = self.pool.begin().await?;
        Ok(Box::new(PgTx::<L> {
            tx: tokio::sync::Mutex::new(tx),
            inverse: self.inverse.clone(),
            _label: std::marker::PhantomData,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conformance::{self, ConformanceLabel};
    use sqlx::postgres::PgPoolOptions;

    /// Build a migrated pool against the test Postgres, or `None` when
    /// `DATABASE_URL_PG` is unset so `cargo test` without a database just skips.
    ///
    /// A dedicated `DATABASE_URL_PG` variable (not `DATABASE_URL`) keeps this
    /// separate from what `cargo sqlx prepare` reads, so preparing the offline
    /// cache and running the live conformance suite don't fight over one URL.
    async fn pool() -> Option<PgPool> {
        let url = std::env::var("DATABASE_URL_PG").ok()?;
        let pool = PgPoolOptions::new()
            .max_connections(8)
            .connect(&url)
            .await
            .expect("connect to DATABASE_URL_PG");
        migrate(&pool).await.expect("migrate test database");
        Some(pool)
    }

    /// Truncate both tables so each conformance check runs against an empty store
    /// (the checks assume a fresh dataset; the SQLite backend gets this from a new
    /// in-memory database, Postgres from a shared database wiped between checks).
    async fn truncate(pool: &PgPool) {
        sqlx::query("TRUNCATE objects, associations")
            .execute(pool)
            .await
            .expect("truncate");
    }

    async fn fresh(pool: &PgPool) -> PgStore<ConformanceLabel> {
        truncate(pool).await;
        PgStore::<ConformanceLabel>::connect(pool.clone())
    }

    /// The full backend-agnostic conformance battery against a live Postgres. The
    /// same suite the SQLite and in-memory backends pass. Skipped (not failed)
    /// when `DATABASE_URL_PG` is unset.
    #[tokio::test]
    async fn pg_store_passes_conformance() {
        let Some(pool) = pool().await else {
            eprintln!("skipping pg_store_passes_conformance: DATABASE_URL_PG not set");
            return;
        };

        conformance::cas_update(&fresh(&pool).await).await;
        conformance::rename_semantics(&fresh(&pool).await).await;
        conformance::case_insensitive_names(&fresh(&pool).await).await;
        conformance::transaction_atomicity(&fresh(&pool).await).await;
        conformance::transaction_commit(&fresh(&pool).await).await;
        conformance::sensitive_blob_roundtrip(&fresh(&pool).await).await;
        conformance::search_object_predicates(&fresh(&pool).await).await;
        conformance::search_object_pagination_filters_completely(&fresh(&pool).await).await;
        conformance::search_namespace_and_filter(&fresh(&pool).await).await;
        conformance::edge_filter_predicates(&fresh(&pool).await).await;
        conformance::edge_filter_pagination_completes(&fresh(&pool).await).await;
        conformance::search_fallback_predicates_agree(&fresh(&pool).await).await;
        conformance::edge_listing_is_recency_ordered(&fresh(&pool).await).await;
        conformance::edge_time_window_selects_range(&fresh(&pool).await).await;
        conformance::edge_target_label_pages_completely(&fresh(&pool).await).await;
        conformance::incoming_edges_listed(&fresh(&pool).await).await;
        conformance::edge_target_id_restriction(&fresh(&pool).await).await;
        conformance::count_edges_matches_list(&fresh(&pool).await).await;

        // Inverse-edge maintenance needs a store configured with the resolver.
        truncate(&pool).await;
        let inv = PgStore::<ConformanceLabel>::connect(pool.clone())
            .with_inverse(conformance::parent_child_inverse);
        conformance::inverse_edges(&inv).await;
    }
}
