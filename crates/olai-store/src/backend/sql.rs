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
//! changing a query or the `migrations/`).
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

use crate::label::Label;
use crate::name::ResourceName;
use crate::object::{Association, Object};
use crate::store::{
    AssociationStore, AssociationStoreReader, ObjectStore, ObjectStoreReader, Precondition,
    StoreExec, StoreTx, Transactional,
};
use crate::{Error, Result};

/// Resolves an edge label to its paired inverse label, if any (see
/// [`InMemoryStore`](crate::InMemoryStore)).
pub type InverseResolver = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

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
    sqlx::migrate!("./migrations")
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

impl From<sqlx::Error> for Error {
    fn from(e: sqlx::Error) -> Self {
        match e {
            sqlx::Error::RowNotFound => Error::NotFound,
            sqlx::Error::Database(db) if db.is_unique_violation() => Error::AlreadyExists,
            other => Error::generic(other.to_string()),
        }
    }
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
        Ok(_) => Error::Conflict,
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

async fn op_list_edges<L: Label>(
    conn: &mut SqliteConnection,
    from_id: Uuid,
    label: &str,
    target_label: Option<L>,
    max_results: Option<usize>,
    page_token: Option<String>,
) -> Result<(Vec<Association<L>>, Option<String>)> {
    let offset = parse_token(page_token)?;
    let limit = max_results.unwrap_or(usize::MAX);
    let fetch = limit.saturating_add(1).min(i64::MAX as usize) as i64;
    let offset_i = offset as i64;
    let from_s = from_id.hyphenated().to_string();
    let rows = sqlx::query!(
        r#"SELECT id AS "id!", from_id, label, to_id, to_label, properties, created_at, updated_at
           FROM associations WHERE from_id = ? AND label = ? ORDER BY id LIMIT ? OFFSET ?"#,
        from_s,
        label,
        fetch,
        offset_i
    )
    .fetch_all(conn)
    .await?;

    let mut edges = rows
        .into_iter()
        .map(|r| {
            Ok(Association {
                id: Uuid::parse_str(&r.id)?,
                from_id: Uuid::parse_str(&r.from_id)?,
                label: r.label,
                to_id: Uuid::parse_str(&r.to_id)?,
                to_label: L::from_str(&r.to_label)
                    .map_err(|_| Error::generic("unknown label in row"))?,
                properties: r.properties.map(|p| serde_json::from_str(&p)).transpose()?,
                created_at: parse_ts(&r.created_at)?,
                updated_at: r.updated_at.as_deref().map(parse_ts).transpose()?,
            })
        })
        .collect::<Result<Vec<Association<L>>>>()?;
    if let Some(tl) = target_label {
        edges.retain(|e| e.to_label == tl);
    }
    paginate(edges, offset, limit)
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
    let id_s = Uuid::new_v4().hyphenated().to_string();
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
    async fn get(&self, id: &Uuid) -> Result<Object<L>> {
        let mut conn = self.pool.acquire().await?;
        op_get(&mut conn, id).await
    }

    async fn get_by_name(&self, label: L, name: &ResourceName) -> Result<Object<L>> {
        let mut conn = self.pool.acquire().await?;
        op_get_by_name(&mut conn, label, name).await
    }

    async fn list(
        &self,
        label: L,
        namespace: Option<&ResourceName>,
        max_results: Option<usize>,
        page_token: Option<String>,
    ) -> Result<(Vec<Object<L>>, Option<String>)> {
        let mut conn = self.pool.acquire().await?;
        op_list_objects(&mut conn, label, namespace, max_results, page_token).await
    }

    async fn get_sensitive(&self, id: &Uuid) -> Result<Option<Bytes>> {
        let mut conn = self.pool.acquire().await?;
        op_get_sensitive(&mut conn, id).await
    }
}

#[async_trait::async_trait]
impl<L: Label> ObjectStore<L> for SqlStore<L> {
    async fn create(
        &self,
        label: L,
        name: &ResourceName,
        properties: Option<serde_json::Value>,
        id: Option<Uuid>,
        sensitive: Option<Bytes>,
    ) -> Result<Object<L>> {
        let mut conn = self.pool.acquire().await?;
        op_create(&mut conn, label, name, properties, id, sensitive).await
    }

    async fn update(
        &self,
        id: &Uuid,
        properties: Option<serde_json::Value>,
        precondition: Precondition,
        sensitive: Option<Bytes>,
    ) -> Result<Object<L>> {
        let mut tx = self.pool.begin().await?;
        let out = op_update(&mut tx, id, properties, precondition, sensitive).await?;
        tx.commit().await?;
        Ok(out)
    }

    async fn rename(
        &self,
        id: &Uuid,
        new_name: &ResourceName,
        precondition: Precondition,
    ) -> Result<Object<L>> {
        let mut tx = self.pool.begin().await?;
        let out = op_rename(&mut tx, id, new_name, precondition).await?;
        tx.commit().await?;
        Ok(out)
    }

    async fn delete(&self, id: &Uuid) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        op_delete(&mut tx, id).await?;
        tx.commit().await?;
        Ok(())
    }

    async fn set_sensitive(&self, id: &Uuid, sensitive: Bytes) -> Result<()> {
        let mut conn = self.pool.acquire().await?;
        op_set_sensitive(&mut conn, id, &sensitive).await
    }
}

#[async_trait::async_trait]
impl<L: Label> AssociationStoreReader<L> for SqlStore<L> {
    async fn list(
        &self,
        from_id: Uuid,
        label: &str,
        target_label: Option<L>,
        max_results: Option<usize>,
        page_token: Option<String>,
    ) -> Result<(Vec<Association<L>>, Option<String>)> {
        let mut conn = self.pool.acquire().await?;
        op_list_edges(
            &mut conn,
            from_id,
            label,
            target_label,
            max_results,
            page_token,
        )
        .await
    }
}

#[async_trait::async_trait]
impl<L: Label> AssociationStore<L> for SqlStore<L> {
    async fn add(
        &self,
        from_id: Uuid,
        to_id: Uuid,
        label: &str,
        properties: Option<serde_json::Value>,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        op_add_edge::<L>(&mut tx, from_id, to_id, label, properties, &self.inverse).await?;
        tx.commit().await?;
        Ok(())
    }

    async fn remove(&self, from_id: Uuid, to_id: Uuid, label: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        op_remove_edge(&mut tx, from_id, to_id, label, &self.inverse).await?;
        tx.commit().await?;
        Ok(())
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
    async fn get(&self, id: &Uuid) -> Result<Object<L>> {
        let mut tx = self.tx.lock().await;
        op_get(&mut tx, id).await
    }

    async fn get_by_name(&self, label: L, name: &ResourceName) -> Result<Object<L>> {
        let mut tx = self.tx.lock().await;
        op_get_by_name(&mut tx, label, name).await
    }

    async fn list(
        &self,
        label: L,
        namespace: Option<&ResourceName>,
        max_results: Option<usize>,
        page_token: Option<String>,
    ) -> Result<(Vec<Object<L>>, Option<String>)> {
        let mut tx = self.tx.lock().await;
        op_list_objects(&mut tx, label, namespace, max_results, page_token).await
    }

    async fn get_sensitive(&self, id: &Uuid) -> Result<Option<Bytes>> {
        let mut tx = self.tx.lock().await;
        op_get_sensitive(&mut tx, id).await
    }
}

#[async_trait::async_trait]
impl<L: Label> ObjectStore<L> for SqlTx<L> {
    async fn create(
        &self,
        label: L,
        name: &ResourceName,
        properties: Option<serde_json::Value>,
        id: Option<Uuid>,
        sensitive: Option<Bytes>,
    ) -> Result<Object<L>> {
        let mut tx = self.tx.lock().await;
        op_create(&mut tx, label, name, properties, id, sensitive).await
    }

    async fn update(
        &self,
        id: &Uuid,
        properties: Option<serde_json::Value>,
        precondition: Precondition,
        sensitive: Option<Bytes>,
    ) -> Result<Object<L>> {
        let mut tx = self.tx.lock().await;
        op_update(&mut tx, id, properties, precondition, sensitive).await
    }

    async fn rename(
        &self,
        id: &Uuid,
        new_name: &ResourceName,
        precondition: Precondition,
    ) -> Result<Object<L>> {
        let mut tx = self.tx.lock().await;
        op_rename(&mut tx, id, new_name, precondition).await
    }

    async fn delete(&self, id: &Uuid) -> Result<()> {
        let mut tx = self.tx.lock().await;
        op_delete(&mut tx, id).await
    }

    async fn set_sensitive(&self, id: &Uuid, sensitive: Bytes) -> Result<()> {
        let mut tx = self.tx.lock().await;
        op_set_sensitive(&mut tx, id, &sensitive).await
    }
}

#[async_trait::async_trait]
impl<L: Label> AssociationStoreReader<L> for SqlTx<L> {
    async fn list(
        &self,
        from_id: Uuid,
        label: &str,
        target_label: Option<L>,
        max_results: Option<usize>,
        page_token: Option<String>,
    ) -> Result<(Vec<Association<L>>, Option<String>)> {
        let mut tx = self.tx.lock().await;
        op_list_edges(
            &mut tx,
            from_id,
            label,
            target_label,
            max_results,
            page_token,
        )
        .await
    }
}

#[async_trait::async_trait]
impl<L: Label> AssociationStore<L> for SqlTx<L> {
    async fn add(
        &self,
        from_id: Uuid,
        to_id: Uuid,
        label: &str,
        properties: Option<serde_json::Value>,
    ) -> Result<()> {
        let inverse = self.inverse.clone();
        let mut tx = self.tx.lock().await;
        op_add_edge::<L>(&mut tx, from_id, to_id, label, properties, &inverse).await
    }

    async fn remove(&self, from_id: Uuid, to_id: Uuid, label: &str) -> Result<()> {
        let inverse = self.inverse.clone();
        let mut tx = self.tx.lock().await;
        op_remove_edge(&mut tx, from_id, to_id, label, &inverse).await
    }
}

#[async_trait::async_trait]
impl<L: Label> StoreTx<L> for SqlTx<L> {
    async fn commit(self: Box<Self>) -> Result<()> {
        self.tx.into_inner().commit().await?;
        Ok(())
    }

    async fn rollback(self: Box<Self>) -> Result<()> {
        self.tx.into_inner().rollback().await?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl<L: Label> Transactional<L> for SqlStore<L> {
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
                let _ = handle.tx.into_inner().rollback().await;
                Err(e)
            }
        }
    }

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
                .create(ConformanceLabel, &ns_name, None, None, None)
                .await
                .unwrap();
            store
                .create(ConformanceLabel, &other, None, None, None)
                .await
                .unwrap();
        }

        let ns = ResourceName::from_naive_str_split("ns");
        let mut seen = Vec::new();
        let mut token = None;
        loop {
            let (page, next) =
                ObjectStoreReader::list(&store, ConformanceLabel, Some(&ns), Some(2), token)
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
            .create(ConformanceLabel, &"m".parse().unwrap(), None, None, None)
            .await
            .unwrap();
        assert!(store.get(&obj.id).await.is_ok());
    }
}
