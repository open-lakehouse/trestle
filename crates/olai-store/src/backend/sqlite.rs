//! SQLite-backed store ([`SqlStore`]).
//!
//! A persistent implementation of [`ObjectStore`], [`AssociationStore`], and
//! [`Transactional`] on top of [`sqlx`] and SQLite. Compare-and-swap
//! ([`Precondition::Version`]) is a real `UPDATE … WHERE version = ?`, and
//! [`transaction`](Transactional::transaction) runs on a genuine database
//! transaction (commit on `Ok`, rollback on `Err`).
//!
//! Enabled by the `sqlite` feature. It runs the shared
//! [conformance](crate::conformance) battery, the same one
//! [`InMemoryStore`](crate::InMemoryStore) passes.
//!
//! SQL is checked at compile time with sqlx's `query!` macros against the
//! committed `.sqlx/` offline cache (regenerate with `cargo sqlx prepare` after
//! changing a query or the `migrations/`). The one exception is
//! [`search`](ObjectStoreReader::search) / [`query_edges`](AssociationStoreReader::query_edges),
//! whose payload-filter `WHERE` clause is built dynamically with [`sqlx::QueryBuilder`] and so is
//! neither compile-time checked nor cached — see the "Filter pushdown" section below.
//!
//! # Migrations
//!
//! Applying the schema is an **explicit** step, decoupled from constructing the
//! store — [`SqlStore::connect`] assumes an already-migrated pool and runs no
//! DDL. This matters for multi-process deployments (e.g. many pods on one
//! Postgres): eager migrate-on-connect would have every process race the
//! migration lock on boot and strips operators of a gated deploy step. Run
//! [`migrate`] / [`migrator`] once as a deliberate step, then
//! [`connect`](SqlStore::connect). For single-process / ephemeral cases (SQLite,
//! tests), [`SqlStore::connect_and_migrate`] and [`SqlStore::in_memory`] bundle
//! migrate-then-connect for convenience.

use std::sync::Arc;

use bytes::Bytes;
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};
use sqlx::{Sqlite, SqliteConnection};
use uuid::Uuid;

use crate::filter::{CompareOp, Filter, Predicate};
use crate::label::Label;
use crate::name::ResourceName;
use crate::object::{Association, Object};
use crate::store::{
    AssociationStore, AssociationStoreReader, EdgeEndpoint, EdgeQuery, ObjectStore,
    ObjectStoreReader, Precondition, StoreExec, StoreTx, Transactional,
};
use crate::{Error, Result};

use super::sql_common::{InverseResolver, merge, record_err};

/// The OpenTelemetry `db.system` value for this backend's operation spans.
const DB_SYSTEM: &str = "sqlite";

/// The crate's embedded schema [`Migrator`](sqlx::migrate::Migrator) for the
/// `SqlStore` backend.
///
/// This is the primary way to apply the store's schema. Migrations are a
/// deliberate, explicit step — [`SqlStore::connect`] does **not** run them for
/// you (see its docs for why eager migrate-on-connect is the wrong default for a
/// multi-process deployment). Run them at deploy time (a release-phase job),
/// apply them inside a larger transaction alongside your own tables, or compose
/// this set with your own. The migrator is embedded at compile time (via
/// [`sqlx::migrate!`]) — it needs no `migrations/` directory at runtime — and can
/// run against anything implementing [`sqlx::Acquire`] (a pool, a connection, or
/// an open transaction):
///
/// ```no_run
/// # async fn run(pool: sqlx::SqlitePool) -> Result<(), Box<dyn std::error::Error>> {
/// olai_store::backend::sql::migrator().run(&pool).await?;
/// # Ok(())
/// # }
/// ```
///
/// `Migrator::run` takes a migration lock (a Postgres advisory lock; a busy-wait
/// on SQLite), so concurrent callers serialize safely — but prefer running
/// migrations once as a gated step over racing them from every process on boot.
pub fn migrator() -> sqlx::migrate::Migrator {
    sqlx::migrate!("./migrations/sqlite")
}

/// Build a single [`Migrator`](sqlx::migrate::Migrator) over this crate's schema
/// migrations **plus** the consumer's own, in one ordered ledger.
///
/// A consumer that stores its own tables in the same database as the object
/// graph (e.g. a secrets or audit table) would otherwise need a *second*
/// `sqlx::migrate!` migrator — but two migrators share sqlx's single hardcoded
/// `_sqlx_migrations` ledger, which forces non-overlapping version ranges and
/// `set_ignore_missing(true)` on both so neither trips over the other's rows.
/// Merging into one migrator sidesteps all of that: one ledger, ordinary
/// version ordering, no `ignore_missing`.
///
/// `extra` is typically the consumer's own `sqlx::migrate!().migrations` (or any
/// iterator of [`Migration`](sqlx::migrate::Migration)). Versions across the
/// combined set must be unique and are applied in ascending version order, so
/// number your migrations above this crate's (which occupy the low range).
///
/// ```no_run
/// # async fn run(pool: sqlx::SqlitePool) -> Result<(), Box<dyn std::error::Error>> {
/// // The consumer's own migrations live in its `./migrations` dir, versioned
/// // above olai-store's (e.g. 0100+).
/// let local = sqlx::migrate!("./migrations");
/// olai_store::sql_migrator_with(local.migrations.iter().cloned())
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

/// Apply the crate's schema migrations to `pool`.
///
/// A convenience wrapper over [`migrator`]. Migrations are idempotent and
/// versioned: running them against an already-current database is a no-op. This
/// is the explicit migration entry point — call it as a deliberate step (e.g. a
/// deploy/release job or test setup), then hand the migrated pool to
/// [`SqlStore::connect`]. For single-process/ephemeral cases where
/// migrate-on-startup is fine, [`SqlStore::connect_and_migrate`] bundles the two.
///
/// # Errors
///
/// Returns a backend error if a migration fails to apply.
pub async fn migrate(pool: &SqlitePool) -> Result<()> {
    migrator()
        .run(pool)
        .await
        .map_err(|e| Error::generic(e.to_string()))
}

/// A SQLite-backed [`ObjectStore`] + [`AssociationStore`] + [`Transactional`].
#[derive(Clone)]
pub struct SqlStore<L: Label> {
    pool: SqlitePool,
    inverse: InverseResolver,
    _label: std::marker::PhantomData<L>,
}

impl<L: Label> SqlStore<L> {
    /// Wrap an existing, **already-migrated** pool.
    ///
    /// This does **not** apply migrations — it assumes the schema is already in
    /// place. Coupling DDL to store construction is the wrong default for a
    /// multi-process deployment (e.g. many pods against one Postgres): every
    /// process would race the migration lock on boot, and operators lose the
    /// ability to run migrations as a deliberate, gated deploy step. Apply the
    /// schema once, explicitly, via [`migrate`] / [`migrator`] (typically a
    /// release-phase job), then call this.
    ///
    /// For a single-process or ephemeral database (SQLite, tests) where
    /// migrate-on-startup is fine, use [`connect_and_migrate`](Self::connect_and_migrate)
    /// or [`in_memory`](Self::in_memory).
    pub fn connect(pool: SqlitePool) -> Self {
        Self {
            pool,
            inverse: Arc::new(|_| None),
            _label: std::marker::PhantomData,
        }
    }

    /// Apply the schema migrations to `pool`, then wrap it.
    ///
    /// A convenience for single-process / ephemeral setups (SQLite, tests) where
    /// migrate-on-startup is acceptable. **Do not** use this as the default in a
    /// multi-process deployment — migrate explicitly and use
    /// [`connect`](Self::connect) instead (see its docs).
    ///
    /// # Errors
    ///
    /// Returns a backend error if a migration fails.
    pub async fn connect_and_migrate(pool: SqlitePool) -> Result<Self> {
        migrate(&pool).await?;
        Ok(Self::connect(pool))
    }

    /// Maintain inverse edges via `resolver`. Chainable onto either constructor.
    #[must_use]
    pub fn with_inverse(
        mut self,
        resolver: impl Fn(&str) -> Option<String> + Send + Sync + 'static,
    ) -> Self {
        self.inverse = Arc::new(resolver);
        self
    }

    /// Open a migrated in-memory SQLite database (handy for tests).
    ///
    /// The pool is pinned to a **single connection**: each physical connection
    /// to `sqlite::memory:` gets its own private database, so a multi-connection
    /// pool would scatter writes across unrelated in-memory databases (and only
    /// migrate one of them). One connection keeps all state coherent.
    ///
    /// # Errors
    ///
    /// Returns a backend error if the connection or migration fails.
    pub async fn in_memory() -> Result<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;
        Self::connect_and_migrate(pool).await
    }
}

// --- Row → domain decoding. The `query!` macros give typed columns; we
//     assemble the generic `Object<L>` / `Association<L>` from them. ---

fn build_object<L: Label>(
    id: String,
    label: String,
    name: String,
    properties: Option<String>,
    version: i64,
    created_at: String,
    updated_at: Option<String>,
) -> Result<Object<L>> {
    Ok(Object {
        id: Uuid::parse_str(&id)?,
        label: L::from_str(&label).map_err(|_| Error::generic("unknown label in row"))?,
        name: name.parse()?,
        properties: properties.map(|p| serde_json::from_str(&p)).transpose()?,
        version: version as u64,
        created_at: parse_ts(&created_at)?,
        updated_at: updated_at.as_deref().map(parse_ts).transpose()?,
    })
}

fn parse_ts(s: &str) -> Result<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .map_err(|e| Error::generic(format!("bad timestamp {s:?}: {e}")))
}

fn json_str(v: &Option<serde_json::Value>) -> Result<Option<String>> {
    v.as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(Into::into)
}

// --- Operations over a single `SqliteConnection`, so the same code runs on a
//     pooled connection (auto-commit) or inside a transaction. ---

async fn op_get<L: Label>(conn: &mut SqliteConnection, id: &Uuid) -> Result<Object<L>> {
    let id = id.hyphenated().to_string();
    let row = sqlx::query!(
        r#"SELECT id AS "id!", label, name, properties, version, created_at, updated_at
           FROM objects WHERE id = ?"#,
        id
    )
    .fetch_optional(conn)
    .await?
    .ok_or(Error::NotFound)?;
    build_object(
        row.id,
        row.label,
        row.name,
        row.properties,
        row.version,
        row.created_at,
        row.updated_at,
    )
}

async fn op_get_by_name<L: Label>(
    conn: &mut SqliteConnection,
    label: L,
    name: &ResourceName,
) -> Result<Object<L>> {
    let label_s = label.as_str().to_string();
    let name_s = name.to_string();
    let row = sqlx::query!(
        r#"SELECT id AS "id!", label, name, properties, version, created_at, updated_at
           FROM objects WHERE label = ? AND name = ?"#,
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
        row.properties,
        row.version,
        row.created_at,
        row.updated_at,
    )
}

async fn op_list_objects<L: Label>(
    conn: &mut SqliteConnection,
    label: L,
    namespace: Option<&ResourceName>,
    max_results: Option<usize>,
    page_token: Option<String>,
) -> Result<(Vec<Object<L>>, Option<String>)> {
    let offset = parse_token(page_token)?;
    let limit = max_results.unwrap_or(usize::MAX);
    let label_s = label.as_str().to_string();

    // The namespace filter is a prefix over the escaped `ResourceName` string,
    // which does not translate to a `LIKE`/`GLOB` the DB can page on. So when a
    // namespace is set, we cannot let SQL `LIMIT`/`OFFSET` truncate before the
    // filter runs — that would drop matching rows and desync the page token.
    // Fetch all label rows and paginate the filtered result in-process; without
    // a namespace, keep the efficient SQL `LIMIT`/`OFFSET` path.
    let (fetch, offset_i) = match namespace {
        Some(_) => (i64::MAX, 0),
        None => (
            limit.saturating_add(1).min(i64::MAX as usize) as i64,
            offset as i64,
        ),
    };
    let rows = sqlx::query!(
        r#"SELECT id AS "id!", label, name, properties, version, created_at, updated_at
           FROM objects WHERE label = ? ORDER BY id LIMIT ? OFFSET ?"#,
        label_s,
        fetch,
        offset_i
    )
    .fetch_all(conn)
    .await?;

    let mut objects = rows
        .into_iter()
        .map(|r| {
            build_object(
                r.id,
                r.label,
                r.name,
                r.properties,
                r.version,
                r.created_at,
                r.updated_at,
            )
        })
        .collect::<Result<Vec<_>>>()?;

    if let Some(ns) = namespace {
        // Filter, then apply offset + limit over the filtered set in-process.
        objects.retain(|o| o.name.prefix_matches(ns));
        let start = offset.min(objects.len());
        objects.drain(..start);
        return paginate(objects, offset, limit);
    }
    paginate(objects, offset, limit)
}

// --- Filter pushdown ------------------------------------------------------------------------
//
// `search` translates the subset of the `Filter` AST that maps *faithfully* to SQLite into a
// `WHERE` clause over `json_extract(properties, '$.path')`, so the database does the filtering
// and real `LIMIT`/`OFFSET` pagination. Anything outside that subset falls back to the trait's
// Rust-side default (drain the full listing, filter with `Filter::matches`) — see
// `op_search_objects` / `op_query_edges`.
//
// The translated query is built with [`sqlx::QueryBuilder`], so it is **not** compile-time
// checked and has **no** `.sqlx` offline-cache entry — the accepted tradeoff for a dynamic
// predicate. Every other query in this backend keeps the checked `sqlx::query!` form. All values
// and JSONPaths are `push_bind`'d; only static column identifiers are written into the SQL text,
// so no user input is ever interpolated.

/// Which predicates translate faithfully to SQLite. The reference evaluator
/// ([`Filter::matches`]) is the source of truth; a predicate is only pushed when its SQL form
/// provably agrees with it.
///
/// Pushable: [`Exists`](Predicate::Exists); and [`Eq`](CompareOp::Eq) / ordered comparisons on a
/// scalar (string / number / bool) value, combined with `And`/`Or`/`Not`. Everything else —
/// `Contains`, `Ne`, and comparisons whose query value is `null`, an array, or an object — is
/// left for the Rust fallback, where matching SQLite's three-valued logic and type coercion to
/// the evaluator would be error-prone.
fn is_pushable(filter: &Filter) -> bool {
    match filter {
        Filter::And(fs) | Filter::Or(fs) => fs.iter().all(is_pushable),
        Filter::Not(f) => is_pushable(f),
        Filter::Predicate(Predicate::Exists { path }) => is_pushable_path(path),
        Filter::Predicate(Predicate::Compare { path, op, value }) => {
            is_pushable_path(path)
                && match op {
                    // `Ne`/`Contains` are deliberately not pushed (see the module comment above).
                    CompareOp::Ne | CompareOp::Contains => false,
                    // Only scalar comparands translate; null/array/object are left to the fallback.
                    CompareOp::Eq
                    | CompareOp::Lt
                    | CompareOp::Le
                    | CompareOp::Gt
                    | CompareOp::Ge => value.is_string() || value.is_number() || value.is_boolean(),
                }
        }
    }
}

/// Whether every segment of `path` is safe to render into a bare `$.a.b` JSONPath.
///
/// [`json_path`] joins segments with `.`, so a segment containing a JSONPath metacharacter
/// (`.`, `[`, `]`, `"`) would be parsed differently by SQLite (e.g. `["a.b"]` → `$.a.b`, read as
/// nested `a`→`b`) and diverge from the reference evaluator, which treats each segment as a
/// literal object key. Such paths are left to the Rust fallback, which resolves them correctly.
fn is_pushable_path(path: &crate::filter::FieldPath) -> bool {
    path.segments()
        .iter()
        .all(|seg| !seg.contains(['.', '[', ']', '"']))
}

/// The JSONPath string for a field path: `["a", "b"]` → `"$.a.b"`. Bound as a parameter to
/// `json_extract` / `json_type`, never interpolated into SQL text.
///
/// Only called for paths that pass [`is_pushable_path`], so no segment contains a JSONPath
/// metacharacter that would change how SQLite parses the path.
fn json_path(path: &crate::filter::FieldPath) -> String {
    let mut s = String::from("$");
    for seg in path.segments() {
        s.push('.');
        s.push_str(seg);
    }
    s
}

/// The `json_type(properties, ?)` values a scalar comparand is allowed to match, mirroring
/// [`crate::filter`]'s `ordering`/`equal`: numbers match `integer`/`real`, strings match `text`,
/// and booleans match `true`/`false`. A row whose value at the path has any other type (or is
/// absent) must not match — matching the evaluator's "type mismatch ⇒ no match".
fn allowed_types(value: &serde_json::Value) -> &'static [&'static str] {
    if value.is_number() {
        &["integer", "real"]
    } else if value.is_string() {
        &["text"]
    } else {
        // booleans (is_pushable already excluded everything else)
        &["true", "false"]
    }
}

/// Emit the SQL comparison operator for a pushable [`CompareOp`]. `Eq` uses `=`; the ordered ops
/// map directly. (`Ne`/`Contains` never reach here — `is_pushable` rejects them.)
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

/// Recursively emit `filter` into `qb` as a boolean SQL expression that evaluates to a definite
/// 0/1 (never SQL `NULL`), so it composes correctly under `AND`/`OR`/`NOT`.
///
/// Precondition: `is_pushable(filter)` is `true`.
fn build_where(qb: &mut sqlx::QueryBuilder<'_, Sqlite>, filter: &Filter) {
    match filter {
        // Empty And ⇒ true (1); empty Or ⇒ false (0); matching the evaluator's identities.
        Filter::And(fs) if fs.is_empty() => {
            qb.push("1");
        }
        Filter::Or(fs) if fs.is_empty() => {
            qb.push("0");
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
            // `build_where` yields a definite 0/1, so plain NOT can't leak NULL.
            qb.push("(NOT ");
            build_where(qb, f);
            qb.push(")");
        }
        Filter::Predicate(Predicate::Exists { path }) => {
            // Present (including JSON null) ⇔ json_type is non-NULL; absent ⇔ NULL.
            qb.push("(json_type(properties, ");
            qb.push_bind(json_path(path));
            qb.push(") IS NOT NULL)");
        }
        Filter::Predicate(Predicate::Compare { path, op, value }) => {
            let p = json_path(path);
            // Two-valued result: type guard AND value comparison, wrapped in COALESCE so an
            // absent path (json_type NULL) yields 0 rather than NULL.
            qb.push("COALESCE((json_type(properties, ");
            qb.push_bind(p.clone());
            qb.push(") IN (");
            for (i, ty) in allowed_types(value).iter().enumerate() {
                if i > 0 {
                    qb.push(", ");
                }
                qb.push_bind(*ty);
            }
            qb.push(")) AND (json_extract(properties, ");
            qb.push_bind(p);
            qb.push(") ");
            qb.push(sql_op(*op));
            qb.push(" ");
            bind_comparand(qb, value);
            qb.push("), 0)");
        }
    }
}

/// Bind a scalar comparand so SQLite compares it in the same domain the evaluator does:
/// numbers as reals, booleans as their `json_extract` integer form (1/0), strings as text.
fn bind_comparand(qb: &mut sqlx::QueryBuilder<'_, Sqlite>, value: &serde_json::Value) {
    match value {
        serde_json::Value::Number(n) => {
            // Compare numerically as f64, matching `filter::ordering`.
            qb.push_bind(n.as_f64().unwrap_or(f64::NAN));
        }
        serde_json::Value::Bool(b) => {
            // `json_extract` returns 1/0 for JSON booleans.
            qb.push_bind(if *b { 1_i64 } else { 0_i64 });
        }
        serde_json::Value::String(s) => {
            qb.push_bind(s.clone());
        }
        // is_pushable guarantees a scalar; other variants never reach here.
        _ => unreachable!("non-scalar comparand is not pushable"),
    }
}

/// Search objects by payload, pushing the filter into SQL when it translates faithfully.
///
/// - Fully pushable **and** no namespace: a single `WHERE label = ? AND <filter>` query with
///   real `LIMIT`/`OFFSET`.
/// - Fully pushable **and** a namespace prefix: push the filter to shrink the scan, but (as in
///   [`op_list_objects`]) fetch every matching row and apply the namespace prefix + pagination
///   in Rust, since the prefix can't be pushed and must not run behind a `LIMIT`.
/// - Not pushable: fall back to the Rust default — list everything and filter with
///   [`Filter::matches`].
async fn op_search_objects<L: Label>(
    conn: &mut SqliteConnection,
    label: L,
    namespace: Option<&ResourceName>,
    filter: &Filter,
    max_results: Option<usize>,
    page_token: Option<String>,
) -> Result<(Vec<Object<L>>, Option<String>)> {
    if !is_pushable(filter) {
        // Rust fallback: drain the full (namespaced) listing, then filter + paginate in process.
        let offset = crate::store::parse_offset(page_token)?;
        let (all, _) = op_list_objects(conn, label, namespace, None, None).await?;
        let matched: Vec<_> = all
            .into_iter()
            .filter(|o| filter.matches(crate::store::props_or_null(&o.properties)))
            .collect();
        return Ok(crate::store::paginate_filtered(
            matched,
            offset,
            max_results,
        ));
    }

    let offset = parse_token(page_token)?;
    let limit = max_results.unwrap_or(usize::MAX);
    let label_s = label.as_str().to_string();

    let mut qb = sqlx::QueryBuilder::<Sqlite>::new(
        r#"SELECT id, label, name, properties, version, created_at, updated_at
           FROM objects WHERE label = "#,
    );
    qb.push_bind(label_s);
    qb.push(" AND ");
    build_where(&mut qb, filter);
    qb.push(" ORDER BY id");

    // Without a namespace, SQL can page directly (over-fetch one row to detect "has more").
    // With a namespace, fetch every filter-matching row and page after the Rust prefix filter.
    if namespace.is_none() {
        let fetch = limit.saturating_add(1).min(i64::MAX as usize) as i64;
        qb.push(" LIMIT ");
        qb.push_bind(fetch);
        qb.push(" OFFSET ");
        qb.push_bind(offset as i64);
    }

    let rows = qb.build().fetch_all(conn).await?;
    let mut objects = rows
        .into_iter()
        .map(object_from_row)
        .collect::<Result<Vec<_>>>()?;

    if let Some(ns) = namespace {
        objects.retain(|o| o.name.prefix_matches(ns));
        let start = offset.min(objects.len());
        objects.drain(..start);
    }
    paginate(objects, offset, limit)
}

/// List edges matching an [`EdgeQuery`], most-recent-first, pushing every predicate it can into
/// one `WHERE`.
///
/// The anchor (`from_id` for [`EdgeEndpoint::From`], `to_id` for [`EdgeEndpoint::Into`]), the edge
/// `label`, an optional `target_label` (`to_label`) and an optional `target_id` (the *opposite*
/// endpoint column) all push into the same `WHERE`, so `LIMIT`/`OFFSET` page correctly. Ordering
/// is `id DESC`: edge ids are UUIDv7 (time-ordered), so descending id is most-recent-first.
///
/// A payload `filter` pushes too when [`is_pushable`]; when it is not, we push everything *else*
/// (so the non-filter predicates still shrink the scan), fetch the full ordered set, and apply
/// the filter + pagination in Rust — never letting a `LIMIT` truncate ahead of the Rust filter.
async fn op_query_edges<L: Label>(
    conn: &mut SqliteConnection,
    query: EdgeQuery<'_, L>,
) -> Result<(Vec<Association<L>>, Option<String>)> {
    // Anchor column + the "other" endpoint column that `target_id` restricts.
    let (anchor_col, other_col, anchor_id) = match query.endpoint {
        EdgeEndpoint::From(id) => ("from_id", "to_id", id),
        EdgeEndpoint::Into(id) => ("to_id", "from_id", id),
    };

    // Emit the anchor + label + target restrictions shared by both paths.
    let base = |qb: &mut sqlx::QueryBuilder<'_, Sqlite>| {
        qb.push(" WHERE ");
        qb.push(anchor_col);
        qb.push(" = ");
        qb.push_bind(anchor_id.hyphenated().to_string());
        qb.push(" AND label = ");
        qb.push_bind(query.label.to_string());
        // `target_label` always filters `to_label` (the only label denormalized on the row);
        // it is meaningful for `From` queries and matches the anchor's own label for `Into`.
        if let Some(tl) = query.target_label {
            qb.push(" AND to_label = ");
            qb.push_bind(tl.as_str().to_string());
        }
        if let Some(tid) = query.target_id {
            qb.push(" AND ");
            qb.push(other_col);
            qb.push(" = ");
            qb.push_bind(tid.hyphenated().to_string());
        }
        // Time window: v7 ids are time-ordered, so a `[since, until)` window on creation time is
        // an id range. `since` inclusive → `id >= since_lo`; `until` exclusive → `id < until_lo`.
        if let Some(since) = query.since {
            qb.push(" AND id >= ");
            qb.push_bind(crate::store::v7_lower_bound(since).hyphenated().to_string());
        }
        if let Some(until) = query.until {
            qb.push(" AND id < ");
            qb.push_bind(crate::store::v7_lower_bound(until).hyphenated().to_string());
        }
    };

    const SELECT: &str = "SELECT id, from_id, label, to_id, to_label, properties, created_at, updated_at \
         FROM associations";

    // Non-pushable filter: push everything else, fetch the full ordered set, filter in Rust.
    if let Some(filter) = query.filter
        && !is_pushable(filter)
    {
        let offset = crate::store::parse_offset(query.page_token)?;
        let mut qb = sqlx::QueryBuilder::<Sqlite>::new(SELECT);
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
        return Ok(crate::store::paginate_filtered(
            matched,
            offset,
            query.max_results,
        ));
    }

    let offset = parse_token(query.page_token)?;
    let limit = query.max_results.unwrap_or(usize::MAX);
    let fetch = limit.saturating_add(1).min(i64::MAX as usize) as i64;

    let mut qb = sqlx::QueryBuilder::<Sqlite>::new(SELECT);
    base(&mut qb);
    if let Some(filter) = query.filter {
        qb.push(" AND ");
        build_where(&mut qb, filter);
    }
    qb.push(" ORDER BY id DESC LIMIT ");
    qb.push_bind(fetch);
    qb.push(" OFFSET ");
    qb.push_bind(offset as i64);

    let rows = qb.build().fetch_all(conn).await?;
    let edges = rows
        .into_iter()
        .map(edge_from_row)
        .collect::<Result<Vec<_>>>()?;
    paginate(edges, offset, limit)
}

/// Count edges matching an anchor + `label` (+ optional `target_label`) via `COUNT(*)`.
async fn op_count_edges<L: Label>(
    conn: &mut SqliteConnection,
    endpoint: EdgeEndpoint,
    label: &str,
    target_label: Option<L>,
) -> Result<u64> {
    let (anchor_col, anchor_id) = match endpoint {
        EdgeEndpoint::From(id) => ("from_id", id),
        EdgeEndpoint::Into(id) => ("to_id", id),
    };
    let mut qb = sqlx::QueryBuilder::<Sqlite>::new("SELECT COUNT(*) FROM associations WHERE ");
    qb.push(anchor_col);
    qb.push(" = ");
    qb.push_bind(anchor_id.hyphenated().to_string());
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
fn object_from_row<L: Label>(row: sqlx::sqlite::SqliteRow) -> Result<Object<L>> {
    use sqlx::Row;
    build_object(
        row.try_get("id")?,
        row.try_get("label")?,
        row.try_get("name")?,
        row.try_get("properties")?,
        row.try_get("version")?,
        row.try_get("created_at")?,
        row.try_get("updated_at")?,
    )
}

/// Build an [`Association`] from a dynamically-queried row.
fn edge_from_row<L: Label>(row: sqlx::sqlite::SqliteRow) -> Result<Association<L>> {
    use sqlx::Row;
    let id: String = row.try_get("id")?;
    let from_id: String = row.try_get("from_id")?;
    let to_id: String = row.try_get("to_id")?;
    let to_label: String = row.try_get("to_label")?;
    let properties: Option<String> = row.try_get("properties")?;
    let created_at: String = row.try_get("created_at")?;
    let updated_at: Option<String> = row.try_get("updated_at")?;
    Ok(Association {
        id: Uuid::parse_str(&id)?,
        from_id: Uuid::parse_str(&from_id)?,
        label: row.try_get("label")?,
        to_id: Uuid::parse_str(&to_id)?,
        to_label: L::from_str(&to_label).map_err(|_| Error::generic("unknown label in row"))?,
        properties: properties.map(|p| serde_json::from_str(&p)).transpose()?,
        created_at: parse_ts(&created_at)?,
        updated_at: updated_at.as_deref().map(parse_ts).transpose()?,
    })
}

async fn op_create<L: Label>(
    conn: &mut SqliteConnection,
    label: L,
    name: &ResourceName,
    properties: Option<serde_json::Value>,
    id: Option<Uuid>,
    sensitive: Option<Bytes>,
) -> Result<Object<L>> {
    let object = Object {
        id: id.unwrap_or_else(Uuid::new_v4),
        label,
        name: name.clone(),
        properties,
        version: 0,
        created_at: chrono::Utc::now(),
        updated_at: None,
    };
    let id_s = object.id.hyphenated().to_string();
    let label_s = object.label.as_str().to_string();
    let name_s = object.name.to_string();
    let props = json_str(&object.properties)?;
    let created = object.created_at.to_rfc3339();
    // The sensitive blob is bound in the same INSERT so the row and its sealed value
    // land atomically (NULL when there is nothing sealed).
    let sensitive = sensitive.as_deref();
    sqlx::query!(
        "INSERT INTO objects (id, label, name, properties, sensitive, version, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, 0, ?, NULL)",
        id_s,
        label_s,
        name_s,
        props,
        sensitive,
        created,
    )
    .execute(conn)
    .await?;
    Ok(object)
}

async fn op_get_sensitive(conn: &mut SqliteConnection, id: &Uuid) -> Result<Option<Bytes>> {
    let id_s = id.hyphenated().to_string();
    // A missing object and an object with no blob both yield `Ok(None)` — matching the
    // `InMemoryStore` backend and the trait contract ("treat `Ok(None)` as no blob regardless
    // of whether the object exists").
    let row = sqlx::query!(r#"SELECT sensitive FROM objects WHERE id = ?"#, id_s)
        .fetch_optional(conn)
        .await?;
    Ok(row.and_then(|r| r.sensitive).map(Bytes::from))
}

/// Replace only the `sensitive` column, leaving properties and version untouched.
async fn op_set_sensitive(conn: &mut SqliteConnection, id: &Uuid, blob: &[u8]) -> Result<()> {
    let id_s = id.hyphenated().to_string();
    let affected = sqlx::query!("UPDATE objects SET sensitive = ? WHERE id = ?", blob, id_s)
        .execute(conn)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(Error::NotFound);
    }
    Ok(())
}

/// A zero-row conditional write means either the row is gone (`NotFound`) or its
/// version moved (`Conflict`). Re-read to disambiguate.
async fn classify_miss<L: Label>(conn: &mut SqliteConnection, id: &Uuid) -> Error {
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
    conn: &mut SqliteConnection,
    id: &Uuid,
    properties: Option<serde_json::Value>,
    precondition: Precondition,
    sensitive: Option<Bytes>,
) -> Result<Object<L>> {
    let id_s = id.hyphenated().to_string();
    let props = json_str(&properties)?;
    let now = chrono::Utc::now().to_rfc3339();
    let blob = sensitive.as_deref();

    // Literal queries keep compile-time checking while supporting the optional CAS guard
    // and the optional sensitive-blob replacement. A `None` blob leaves the stored
    // `sensitive` column untouched; a `Some` blob replaces it in the same statement so the
    // properties and the sealed value update atomically.
    let affected = match (precondition, blob) {
        (Precondition::Any, None) => sqlx::query!(
            "UPDATE objects SET properties = ?, version = version + 1, updated_at = ? \
                 WHERE id = ?",
            props,
            now,
            id_s
        )
        .execute(&mut *conn)
        .await?
        .rows_affected(),
        (Precondition::Any, Some(blob)) => sqlx::query!(
            "UPDATE objects SET properties = ?, sensitive = ?, version = version + 1, updated_at = ? \
                 WHERE id = ?",
            props,
            blob,
            now,
            id_s
        )
        .execute(&mut *conn)
        .await?
        .rows_affected(),
        (Precondition::Version(v), None) => {
            let v = v as i64;
            sqlx::query!(
                "UPDATE objects SET properties = ?, version = version + 1, updated_at = ? \
                 WHERE id = ? AND version = ?",
                props,
                now,
                id_s,
                v
            )
            .execute(&mut *conn)
            .await?
            .rows_affected()
        }
        (Precondition::Version(v), Some(blob)) => {
            let v = v as i64;
            sqlx::query!(
                "UPDATE objects SET properties = ?, sensitive = ?, version = version + 1, updated_at = ? \
                 WHERE id = ? AND version = ?",
                props,
                blob,
                now,
                id_s,
                v
            )
            .execute(&mut *conn)
            .await?
            .rows_affected()
        }
    };
    if affected == 0 {
        return Err(classify_miss::<L>(conn, id).await);
    }
    op_get(conn, id).await
}

async fn op_rename<L: Label>(
    conn: &mut SqliteConnection,
    id: &Uuid,
    new_name: &ResourceName,
    precondition: Precondition,
) -> Result<Object<L>> {
    let id_s = id.hyphenated().to_string();
    let name_s = new_name.to_string();
    let now = chrono::Utc::now().to_rfc3339();

    let affected = match precondition {
        Precondition::Any => sqlx::query!(
            "UPDATE objects SET name = ?, version = version + 1, updated_at = ? WHERE id = ?",
            name_s,
            now,
            id_s
        )
        .execute(&mut *conn)
        .await?
        .rows_affected(),
        Precondition::Version(v) => {
            let v = v as i64;
            sqlx::query!(
                "UPDATE objects SET name = ?, version = version + 1, updated_at = ? \
                 WHERE id = ? AND version = ?",
                name_s,
                now,
                id_s,
                v
            )
            .execute(&mut *conn)
            .await?
            .rows_affected()
        }
    };
    if affected == 0 {
        return Err(classify_miss::<L>(conn, id).await);
    }
    op_get(conn, id).await
}

async fn op_delete(conn: &mut SqliteConnection, id: &Uuid) -> Result<()> {
    let id_s = id.hyphenated().to_string();
    // Cascade edges (either direction), then the object.
    sqlx::query!(
        "DELETE FROM associations WHERE from_id = ? OR to_id = ?",
        id_s,
        id_s
    )
    .execute(&mut *conn)
    .await?;
    let affected = sqlx::query!("DELETE FROM objects WHERE id = ?", id_s)
        .execute(&mut *conn)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(Error::NotFound);
    }
    Ok(())
}

async fn op_add_edge<L: Label>(
    conn: &mut SqliteConnection,
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
    conn: &mut SqliteConnection,
    from_id: Uuid,
    to_id: Uuid,
    label: &str,
    to_label: L,
    properties: Option<serde_json::Value>,
) -> Result<()> {
    // v7 (time-ordered) so `ORDER BY id DESC` in `op_query_edges` is most-recent-first.
    let id_s = Uuid::now_v7().hyphenated().to_string();
    let from_s = from_id.hyphenated().to_string();
    let to_s = to_id.hyphenated().to_string();
    let to_label_s = to_label.as_str().to_string();
    let props = json_str(&properties)?;
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query!(
        "INSERT INTO associations \
         (id, from_id, label, to_id, to_label, properties, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, NULL)",
        id_s,
        from_s,
        label,
        to_s,
        to_label_s,
        props,
        now
    )
    .execute(conn)
    .await?;
    Ok(())
}

async fn op_remove_edge(
    conn: &mut SqliteConnection,
    from_id: Uuid,
    to_id: Uuid,
    label: &str,
    inverse: &InverseResolver,
) -> Result<()> {
    let from_s = from_id.hyphenated().to_string();
    let to_s = to_id.hyphenated().to_string();
    let affected = sqlx::query!(
        "DELETE FROM associations WHERE from_id = ? AND to_id = ? AND label = ?",
        from_s,
        to_s,
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
            "DELETE FROM associations WHERE from_id = ? AND to_id = ? AND label = ?",
            to_s,
            from_s,
            inv
        )
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

fn parse_token(token: Option<String>) -> Result<usize> {
    match token {
        Some(t) => t
            .parse()
            .map_err(|_| Error::invalid_argument("invalid page token")),
        None => Ok(0),
    }
}

/// Trim the over-fetched extra row and compute the next token.
fn paginate<T>(mut items: Vec<T>, offset: usize, limit: usize) -> Result<(Vec<T>, Option<String>)> {
    let has_more = items.len() > limit;
    if has_more {
        items.truncate(limit);
    }
    let next = has_more.then(|| (offset + limit).to_string());
    Ok((items, next))
}

// --- Top-level store: acquire a connection (or open a txn for multi-statement
//     ops) and delegate to the shared operations. ---

#[async_trait::async_trait]
impl<L: Label> ObjectStoreReader<L> for SqlStore<L> {
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
        let mut conn = self.pool.acquire().await?;
        let out = op_get(&mut conn, id).await;
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
        let mut conn = self.pool.acquire().await?;
        let out = op_get_by_name(&mut conn, label, name).await;
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
        let mut conn = self.pool.acquire().await?;
        let out = op_list_objects(&mut conn, label, namespace, max_results, page_token).await;
        record_err(&out);
        out
    }

    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.search",
            otel.kind = "client",
            db.system = DB_SYSTEM,
            db.operation.name = "search",
            db.collection.name = label.as_str(),
            max_results = ?max_results,
            otel.status_code = tracing::field::Empty,
            error.type = tracing::field::Empty,
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
        let mut conn = self.pool.acquire().await?;
        let out = op_get_sensitive(&mut conn, id).await;
        record_err(&out);
        out
    }
}

#[async_trait::async_trait]
impl<L: Label> ObjectStore<L> for SqlStore<L> {
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
        let mut conn = self.pool.acquire().await?;
        let out = op_create(&mut conn, label, name, properties, id, sensitive).await;
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
        let mut conn = self.pool.acquire().await?;
        let out = op_set_sensitive(&mut conn, id, &sensitive).await;
        record_err(&out);
        out
    }
}

#[async_trait::async_trait]
impl<L: Label> AssociationStoreReader<L> for SqlStore<L> {
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
        let mut conn = self.pool.acquire().await?;
        let out = op_query_edges(&mut conn, query).await;
        record_err(&out);
        out
    }

    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.count_edges",
            otel.kind = "client",
            db.system = DB_SYSTEM,
            db.operation.name = "count_edges",
            label = %label,
            otel.status_code = tracing::field::Empty,
            error.type = tracing::field::Empty,
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
impl<L: Label> AssociationStore<L> for SqlStore<L> {
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

/// An open SQLite transaction handle. Buffers writes until
/// [`commit`](StoreTx::commit); dropping rolls back.
pub struct SqlTx<L: Label> {
    tx: tokio::sync::Mutex<sqlx::Transaction<'static, Sqlite>>,
    inverse: InverseResolver,
    _label: std::marker::PhantomData<L>,
}

#[async_trait::async_trait]
impl<L: Label> ObjectStoreReader<L> for SqlTx<L> {
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
        let mut tx = self.tx.lock().await;
        let out = op_get(&mut tx, id).await;
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
        let mut tx = self.tx.lock().await;
        let out = op_get_by_name(&mut tx, label, name).await;
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
        let mut tx = self.tx.lock().await;
        let out = op_list_objects(&mut tx, label, namespace, max_results, page_token).await;
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
        let mut tx = self.tx.lock().await;
        let out = op_get_sensitive(&mut tx, id).await;
        record_err(&out);
        out
    }
}

#[async_trait::async_trait]
impl<L: Label> ObjectStore<L> for SqlTx<L> {
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
        let mut tx = self.tx.lock().await;
        let out = op_create(&mut tx, label, name, properties, id, sensitive).await;
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
        let mut tx = self.tx.lock().await;
        let out = op_update(&mut tx, id, properties, precondition, sensitive).await;
        record_err(&out);
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
        let mut tx = self.tx.lock().await;
        let out = op_rename(&mut tx, id, new_name, precondition).await;
        record_err(&out);
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
        let mut tx = self.tx.lock().await;
        let out = op_delete(&mut tx, id).await;
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
        let mut tx = self.tx.lock().await;
        let out = op_set_sensitive(&mut tx, id, &sensitive).await;
        record_err(&out);
        out
    }
}

#[async_trait::async_trait]
impl<L: Label> AssociationStoreReader<L> for SqlTx<L> {
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
        let mut tx = self.tx.lock().await;
        let out = op_query_edges(&mut tx, query).await;
        record_err(&out);
        out
    }

    #[tracing::instrument(
        skip_all,
        fields(
            otel.name = "olai_store.count_edges",
            otel.kind = "client",
            db.system = DB_SYSTEM,
            db.operation.name = "count_edges",
            label = %label,
            otel.status_code = tracing::field::Empty,
            error.type = tracing::field::Empty,
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
impl<L: Label> AssociationStore<L> for SqlTx<L> {
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
        let inverse = self.inverse.clone();
        let mut tx = self.tx.lock().await;
        let out = op_add_edge::<L>(&mut tx, from_id, to_id, label, properties, &inverse).await;
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
        let inverse = self.inverse.clone();
        let mut tx = self.tx.lock().await;
        let out = op_remove_edge(&mut tx, from_id, to_id, label, &inverse).await;
        record_err(&out);
        out
    }
}

#[async_trait::async_trait]
impl<L: Label> StoreTx<L> for SqlTx<L> {
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
impl<L: Label> Transactional<L> for SqlStore<L> {
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
        let handle = SqlTx::<L> {
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
        Ok(Box::new(SqlTx::<L> {
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

    async fn fresh() -> SqlStore<ConformanceLabel> {
        SqlStore::in_memory().await.unwrap()
    }

    #[tokio::test]
    async fn sql_store_passes_conformance() {
        // Each check gets its own fresh in-memory DB (they don't share a pool).
        conformance::cas_update(&fresh().await).await;
        conformance::rename_semantics(&fresh().await).await;
        conformance::transaction_atomicity(&fresh().await).await;
        conformance::transaction_commit(&fresh().await).await;
        conformance::sensitive_blob_roundtrip(&fresh().await).await;
        conformance::search_object_predicates(&fresh().await).await;
        conformance::search_object_pagination_filters_completely(&fresh().await).await;
        conformance::search_namespace_and_filter(&fresh().await).await;
        conformance::edge_filter_predicates(&fresh().await).await;
        conformance::edge_filter_pagination_completes(&fresh().await).await;
        conformance::search_fallback_predicates_agree(&fresh().await).await;
        conformance::edge_listing_is_recency_ordered(&fresh().await).await;
        conformance::edge_time_window_selects_range(&fresh().await).await;
        conformance::edge_target_label_pages_completely(&fresh().await).await;
        conformance::incoming_edges_listed(&fresh().await).await;
        conformance::edge_target_id_restriction(&fresh().await).await;
        conformance::count_edges_matches_list(&fresh().await).await;

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let inv = SqlStore::<ConformanceLabel>::connect_and_migrate(pool)
            .await
            .unwrap()
            .with_inverse(conformance::parent_child_inverse);
        conformance::inverse_edges(&inv).await;
    }

    /// A stale-version conditional write is disambiguated as a conflict and surfaced as a
    /// `debug` event (via `classify_miss`), not swallowed.
    #[tokio::test]
    #[tracing_test::traced_test]
    async fn cas_conflict_emits_debug() {
        use crate::name::ResourceName;
        let store = fresh().await;
        let name = ResourceName::from_naive_str_split("a");
        let obj = store
            .create(ConformanceLabel::Node, &name, None, None, None)
            .await
            .unwrap();
        // Bump to version 1, then re-issue the stale (version 0) precondition.
        store
            .update(&obj.id, None, Precondition::Version(0), None)
            .await
            .unwrap();
        let err = store
            .update(&obj.id, None, Precondition::Version(0), None)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Conflict));
        assert!(logs_contain("CAS precondition conflict"));
    }

    /// Paging a namespace-filtered listing must not drop matching rows even when
    /// non-matching rows are interleaved (the filter runs after SQL LIMIT would
    /// have truncated).
    #[tokio::test]
    async fn namespace_filtered_listing_pages_completely() {
        use crate::name::ResourceName;
        let store = fresh().await;
        // Interleave matching (ns.*) and non-matching (other.*) names so a naive
        // SQL LIMIT before filtering would lose matches.
        for i in 0..6 {
            let ns_name = ResourceName::from_naive_str_split(format!("ns.item{i}"));
            let other = ResourceName::from_naive_str_split(format!("other.item{i}"));
            store
                .create(ConformanceLabel::Node, &ns_name, None, None, None)
                .await
                .unwrap();
            store
                .create(ConformanceLabel::Node, &other, None, None, None)
                .await
                .unwrap();
        }

        let ns = ResourceName::from_naive_str_split("ns");
        let mut seen = Vec::new();
        let mut token = None;
        loop {
            let (page, next) =
                ObjectStoreReader::list(&store, ConformanceLabel::Node, Some(&ns), Some(2), token)
                    .await
                    .unwrap();
            assert!(page.iter().all(|o| o.name.prefix_matches(&ns)));
            seen.extend(page.into_iter().map(|o| o.id));
            match next {
                Some(t) => token = Some(t),
                None => break,
            }
        }
        // All six ns.* objects must be returned exactly once.
        assert_eq!(seen.len(), 6, "every namespaced object must be paged");
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), 6, "no duplicates across pages");
    }

    /// The public migration API applies the schema to a caller-supplied pool,
    /// so a consumer can migrate independently and then build the store on the
    /// same, already-migrated pool. Migrations are idempotent — running twice is
    /// a no-op.
    #[tokio::test]
    async fn public_migrate_api_is_reusable_and_idempotent() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();

        // Caller runs migrations themselves, before constructing any store.
        migrate(&pool).await.unwrap();
        // Idempotent: a second run against the current schema is a no-op.
        migrate(&pool).await.unwrap();
        // And the embedded migrator is reachable for advanced composition.
        migrator().run(&pool).await.unwrap();

        // Building the store on the already-migrated pool needs no further
        // migration: `connect` does not run DDL.
        let store = SqlStore::<ConformanceLabel>::connect(pool);
        let obj = store
            .create(
                ConformanceLabel::Node,
                &"m".parse().unwrap(),
                None,
                None,
                None,
            )
            .await
            .unwrap();
        assert!(store.get(&obj.id).await.is_ok());
    }
}
