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
//! changing a query or the `migrations/`). The schema is applied at runtime by
//! [`sqlx::migrate!`].

use std::sync::Arc;

use sqlx::sqlite::SqlitePool;
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
    /// Wrap an existing pool and apply the schema migrations.
    ///
    /// # Errors
    ///
    /// Returns a backend error if the migration fails.
    pub async fn connect(pool: SqlitePool) -> Result<Self> {
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .map_err(|e| Error::generic(e.to_string()))?;
        Ok(Self {
            pool,
            inverse: Arc::new(|_| None),
            _label: std::marker::PhantomData,
        })
    }

    /// Like [`connect`](Self::connect) but maintains inverse edges via `resolver`.
    ///
    /// # Errors
    ///
    /// Returns a backend error if the migration fails.
    pub async fn connect_with_inverse(
        pool: SqlitePool,
        resolver: impl Fn(&str) -> Option<String> + Send + Sync + 'static,
    ) -> Result<Self> {
        let mut this = Self::connect(pool).await?;
        this.inverse = Arc::new(resolver);
        Ok(this)
    }

    /// Open an in-memory SQLite database (handy for tests).
    ///
    /// # Errors
    ///
    /// Returns a backend error if the connection or migration fails.
    pub async fn in_memory() -> Result<Self> {
        let pool = SqlitePool::connect("sqlite::memory:").await?;
        Self::connect(pool).await
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
    let fetch = limit.saturating_add(1).min(i64::MAX as usize) as i64;
    let offset_i = offset as i64;
    let label_s = label.as_str().to_string();
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
        objects.retain(|o| o.name.prefix_matches(ns));
    }
    paginate(objects, offset, limit)
}

async fn op_create<L: Label>(
    conn: &mut SqliteConnection,
    label: L,
    name: &ResourceName,
    properties: Option<serde_json::Value>,
    id: Option<Uuid>,
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
    sqlx::query!(
        "INSERT INTO objects (id, label, name, properties, version, created_at, updated_at) \
         VALUES (?, ?, ?, ?, 0, ?, NULL)",
        id_s,
        label_s,
        name_s,
        props,
        created,
    )
    .execute(conn)
    .await?;
    Ok(object)
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
) -> Result<Object<L>> {
    let id_s = id.hyphenated().to_string();
    let props = json_str(&properties)?;
    let now = chrono::Utc::now().to_rfc3339();

    // Two literal queries keep compile-time checking while supporting the
    // optional CAS guard.
    let affected = match precondition {
        Precondition::Any => sqlx::query!(
            "UPDATE objects SET properties = ?, version = version + 1, updated_at = ? \
                 WHERE id = ?",
            props,
            now,
            id_s
        )
        .execute(&mut *conn)
        .await?
        .rows_affected(),
        Precondition::Version(v) => {
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
}

#[async_trait::async_trait]
impl<L: Label> ObjectStore<L> for SqlStore<L> {
    async fn create(
        &self,
        label: L,
        name: &ResourceName,
        properties: Option<serde_json::Value>,
        id: Option<Uuid>,
    ) -> Result<Object<L>> {
        let mut conn = self.pool.acquire().await?;
        op_create(&mut conn, label, name, properties, id).await
    }

    async fn update(
        &self,
        id: &Uuid,
        properties: Option<serde_json::Value>,
        precondition: Precondition,
    ) -> Result<Object<L>> {
        let mut tx = self.pool.begin().await?;
        let out = op_update(&mut tx, id, properties, precondition).await?;
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
}

#[async_trait::async_trait]
impl<L: Label> ObjectStore<L> for SqlTx<L> {
    async fn create(
        &self,
        label: L,
        name: &ResourceName,
        properties: Option<serde_json::Value>,
        id: Option<Uuid>,
    ) -> Result<Object<L>> {
        let mut tx = self.tx.lock().await;
        op_create(&mut tx, label, name, properties, id).await
    }

    async fn update(
        &self,
        id: &Uuid,
        properties: Option<serde_json::Value>,
        precondition: Precondition,
    ) -> Result<Object<L>> {
        let mut tx = self.tx.lock().await;
        op_update(&mut tx, id, properties, precondition).await
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

        let inv = SqlStore::<ConformanceLabel>::connect_with_inverse(
            SqlitePool::connect("sqlite::memory:").await.unwrap(),
            conformance::parent_child_inverse,
        )
        .await
        .unwrap();
        conformance::inverse_edges(&inv).await;
    }
}
