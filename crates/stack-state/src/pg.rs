//! The shared tables on a team's Postgres database (feature `postgres`).
//!
//! The store's API is synchronous and its callers often run inside a Tokio runtime (the
//! desktop app, `olai-stack-runtime`), where blocking on a future panics. So a
//! [`PgSession`] runs sqlx on a thread of its own, with a current-thread runtime, and each
//! call ships a job there and waits for the reply on a plain channel. A transaction pins one
//! pooled connection on that thread between `BEGIN` and `COMMIT`; the caller keeps the
//! session locked for the whole of it.
//!
//! The pool reconnects on its own, so a session outlives the database's connections being
//! closed under it (Neon closes them when a compute scales to zero).

use std::sync::mpsc;
use std::time::Duration;

use sqlx::pool::PoolConnection;
use sqlx::postgres::{PgArguments, PgConnectOptions, PgPool, PgPoolOptions, PgRow};
use sqlx::{Column, Postgres, Row as _, TypeInfo};
use tokio::runtime::Runtime;

use crate::error::{Error, Result};
use crate::sql::{DbError, Row, Sql, Value};

/// The migrations of the shared schema on Postgres, in order: migration `n` (1-based) takes
/// it from version `n - 1` to `n`. They're separate from SQLite's: the local `state.db` holds
/// more (instances, leases, attachments) and has its own history.
const MIGRATIONS: &[&str] = &[include_str!("../migrations/postgres/0001_init.sql")];

/// The version of the shared schema on Postgres this crate writes and reads.
pub const REMOTE_SCHEMA_VERSION: u32 = MIGRATIONS.len() as u32;

/// Any number taken as the key of the advisory lock migrations hold, so two processes
/// initializing one database don't both migrate it.
const MIGRATION_LOCK: i64 = 0x7472_6573_746c_6501;

/// How long a call waits for a pooled connection (a scaled-to-zero compute takes seconds to
/// wake).
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(30);

type Job = Box<dyn FnOnce(&Runtime, &mut Worker) + Send>;

/// What the session's thread holds.
struct Worker {
    pool: PgPool,
    /// The connection of the open transaction, if one is open.
    tx: Option<PoolConnection<Postgres>>,
}

/// A connection pool to one Postgres database, driven from its own thread.
pub(crate) struct PgSession {
    jobs: mpsc::Sender<Job>,
}

impl std::fmt::Debug for PgSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PgSession").finish_non_exhaustive()
    }
}

fn remote(e: impl std::error::Error + Send + Sync + 'static) -> Error {
    Error::Remote(Box::new(e))
}

fn closed() -> Error {
    Error::Remote("the Postgres session's thread is gone".into())
}

fn pg_error(e: sqlx::Error) -> DbError {
    let kind = e.as_database_error().map(|d| d.kind());
    let err = remote(e);
    match kind {
        Some(sqlx::error::ErrorKind::UniqueViolation) => DbError::Unique(err),
        Some(sqlx::error::ErrorKind::ForeignKeyViolation) => DbError::ForeignKey(err),
        _ => DbError::Other(err),
    }
}

impl PgSession {
    /// Connect to the database at `url` (a `postgresql://` connection string), failing when it
    /// can't be reached.
    pub(crate) fn connect(url: &str) -> Result<Self> {
        let options: PgConnectOptions = url.parse().map_err(remote)?;
        let (jobs, inbox) = mpsc::channel::<Job>();
        let (ready, started) = mpsc::sync_channel::<Result<()>>(1);
        std::thread::Builder::new()
            .name("olai-stack-state-pg".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(e) => {
                        let _ = ready.send(Err(Error::Remote(Box::new(e))));
                        return;
                    }
                };
                // A pooled connection returns itself to the pool from a task it spawns when
                // dropped, outside `block_on` too.
                let _context = runtime.enter();
                let pool = runtime.block_on(
                    PgPoolOptions::new()
                        .max_connections(2)
                        .min_connections(0)
                        .acquire_timeout(ACQUIRE_TIMEOUT)
                        .idle_timeout(Duration::from_secs(60))
                        .connect_with(options),
                );
                let mut worker = match pool {
                    Ok(pool) => Worker { pool, tx: None },
                    Err(e) => {
                        let _ = ready.send(Err(remote(e)));
                        return;
                    }
                };
                let _ = ready.send(Ok(()));
                while let Ok(job) = inbox.recv() {
                    job(&runtime, &mut worker);
                }
                drop(worker.tx.take());
                runtime.block_on(worker.pool.close());
            })
            .map_err(|e| Error::Remote(Box::new(e)))?;
        started.recv().map_err(|_| closed())??;
        Ok(PgSession { jobs })
    }

    /// Run `f` on the session's thread and wait for what it returns.
    fn call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&Runtime, &mut Worker) -> T + Send + 'static,
    ) -> Result<T> {
        let (reply, answer) = mpsc::sync_channel(1);
        self.jobs
            .send(Box::new(move |runtime, worker| {
                let _ = reply.send(f(runtime, worker));
            }))
            .map_err(|_| closed())?;
        answer.recv().map_err(|_| closed())
    }

    /// Open a transaction on a connection of its own; every statement runs on it until
    /// [`end`](Self::end).
    pub(crate) fn begin(&self) -> Result<()> {
        self.call(|runtime, worker| {
            // A transaction left open (its caller panicked before ending it) never goes back
            // to the pool, where its state would leak into the next statement.
            if let Some(stale) = worker.tx.take() {
                drop(stale.detach());
            }
            runtime.block_on(async {
                let mut conn = worker.pool.acquire().await?;
                sqlx::raw_sql("BEGIN").execute(&mut *conn).await?;
                worker.tx = Some(conn);
                Ok::<_, sqlx::Error>(())
            })
        })?
        .map_err(remote)
    }

    /// Commit (`commit`) or roll back the open transaction.
    pub(crate) fn end(&self, commit: bool) -> Result<()> {
        self.call(move |runtime, worker| {
            let Some(mut conn) = worker.tx.take() else {
                return Ok(());
            };
            let result = runtime.block_on(async {
                sqlx::raw_sql(if commit { "COMMIT" } else { "ROLLBACK" })
                    .execute(&mut *conn)
                    .await
            });
            if result.is_err() {
                // Don't hand a connection in an unknown transaction state back to the pool.
                drop(conn.detach());
            }
            result.map(|_| ())
        })?
        .map_err(remote)
    }

    /// Run `sql` as is (several statements, no placeholders) on the open transaction, or on
    /// its own.
    fn batch(&self, sql: &'static str) -> Result<()> {
        self.call(move |runtime, worker| {
            runtime.block_on(async {
                match worker.tx.as_mut() {
                    Some(conn) => sqlx::raw_sql(sql).execute(&mut **conn).await,
                    None => sqlx::raw_sql(sql).execute(&worker.pool).await,
                }
            })
        })?
        .map(|_| ())
        .map_err(remote)
    }
}

fn bind(
    mut query: sqlx::query::Query<'_, Postgres, PgArguments>,
    args: Vec<Value>,
) -> sqlx::query::Query<'_, Postgres, PgArguments> {
    for arg in args {
        query = match arg {
            Value::Null => query.bind(None::<String>),
            Value::NullInt => query.bind(None::<i64>),
            Value::Int(n) => query.bind(n),
            Value::Bool(b) => query.bind(b),
            Value::Text(s) => query.bind(s),
        };
    }
    query
}

fn decode(row: &PgRow) -> std::result::Result<Row, sqlx::Error> {
    let mut values = Vec::with_capacity(row.columns().len());
    for (i, column) in row.columns().iter().enumerate() {
        let value = match column.type_info().name() {
            "BOOL" => row.try_get::<Option<bool>, _>(i)?.map(Value::Bool),
            "INT2" => row
                .try_get::<Option<i16>, _>(i)?
                .map(|n| Value::Int(n.into())),
            "INT4" => row
                .try_get::<Option<i32>, _>(i)?
                .map(|n| Value::Int(n.into())),
            "INT8" => row.try_get::<Option<i64>, _>(i)?.map(Value::Int),
            _ => row.try_get::<Option<String>, _>(i)?.map(Value::Text),
        };
        values.push(value.unwrap_or(Value::Null));
    }
    Ok(Row(values))
}

impl Sql for &PgSession {
    fn execute(&mut self, sql: &str, args: Vec<Value>) -> Result<u64, DbError> {
        let sql = sql.to_owned();
        self.call(move |runtime, worker| {
            runtime.block_on(async {
                let query = bind(sqlx::query(&sql), args);
                match worker.tx.as_mut() {
                    Some(conn) => query.execute(&mut **conn).await,
                    None => query.execute(&worker.pool).await,
                }
                .map(|done| done.rows_affected())
            })
        })?
        .map_err(pg_error)
    }

    fn query(&mut self, sql: &str, args: Vec<Value>) -> Result<Vec<Row>, DbError> {
        let sql = sql.to_owned();
        self.call(move |runtime, worker| {
            runtime.block_on(async {
                let query = bind(sqlx::query(&sql), args);
                let rows = match worker.tx.as_mut() {
                    Some(conn) => query.fetch_all(&mut **conn).await,
                    None => query.fetch_all(&worker.pool).await,
                }?;
                rows.iter()
                    .map(decode)
                    .collect::<std::result::Result<Vec<Row>, sqlx::Error>>()
            })
        })?
        .map_err(pg_error)
    }
}

/// The shared schema's version on the database: `None` when it has none yet (it was never
/// [`migrate`]d).
pub(crate) fn schema_version(session: &PgSession) -> Result<Option<u32>> {
    let mut sql = session;
    let exists = sql
        .query_opt("SELECT to_regclass('state_meta') IS NOT NULL", vec![])?
        .map(|r| r.bool(0))
        .transpose()?
        .unwrap_or(false);
    if !exists {
        return Ok(None);
    }
    let row = sql.query_opt(
        "SELECT value FROM state_meta WHERE key = 'schema_version'",
        vec![],
    )?;
    row.map(|r| r.u32(0)).transpose()
}

/// Check the shared schema is the one this crate reads.
pub(crate) fn check_version(session: &PgSession) -> Result<()> {
    match schema_version(session)? {
        None => Err(Error::Uninitialized),
        Some(found) if found > REMOTE_SCHEMA_VERSION => Err(Error::UnsupportedVersion {
            found,
            supported: REMOTE_SCHEMA_VERSION,
        }),
        Some(found) if found < REMOTE_SCHEMA_VERSION => Err(Error::Outdated {
            found,
            current: REMOTE_SCHEMA_VERSION,
        }),
        Some(_) => Ok(()),
    }
}

/// Create the shared schema, or bring it up to [`REMOTE_SCHEMA_VERSION`], in one transaction
/// holding an advisory lock. Refuses a newer schema.
pub(crate) fn migrate(session: &PgSession) -> Result<()> {
    session.begin()?;
    let result = (|| {
        let mut sql = session;
        sql.execute(
            "SELECT pg_advisory_xact_lock($1)",
            vec![Value::Int(MIGRATION_LOCK)],
        )?;
        let found = schema_version(session)?.unwrap_or(0);
        if found > REMOTE_SCHEMA_VERSION {
            return Err(Error::UnsupportedVersion {
                found,
                supported: REMOTE_SCHEMA_VERSION,
            });
        }
        for migration in &MIGRATIONS[found as usize..] {
            session.batch(migration)?;
        }
        sql.execute(
            "UPDATE state_meta SET value = $1 WHERE key = 'schema_version'",
            vec![Value::from(REMOTE_SCHEMA_VERSION)],
        )?;
        Ok(())
    })();
    session.end(result.is_ok())?;
    result
}

/// The change counter every write to the shared tables bumps.
pub(crate) fn change_seq(session: &PgSession) -> Result<i64> {
    let mut sql = session;
    sql.query_opt(
        "SELECT value FROM state_meta WHERE key = 'change_seq'",
        vec![],
    )?
    .ok_or(Error::Uninitialized)?
    .int(0)
}
