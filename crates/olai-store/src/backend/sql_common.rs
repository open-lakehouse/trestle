//! Glue shared by the sqlx-backed backends ([`sqlite`](super::sqlite) and
//! [`postgres`](super::postgres)).
//!
//! Everything here is **dialect-independent**: it never names a concrete
//! `sqlx::Database`, so it is defined exactly once regardless of which (or both)
//! of the `sqlite` / `postgres` features are enabled. The SQL-bearing code — the
//! `op_*` functions with their compile-time-checked `sqlx::query!` literals, which
//! *are* dialect-bound — lives in each backend module.

use std::sync::Arc;

use crate::{Error, Result};

/// The current span's OpenTelemetry status/error fields, recorded on failure.
///
/// The span must declare `otel.status_code` and `error.type` as
/// [`tracing::field::Empty`]. Only the error *kind* ([`Error::kind_str`]) is
/// recorded — never a payload or message body.
pub(crate) fn record_err<T>(result: &Result<T>) {
    if let Err(e) = result {
        let span = tracing::Span::current();
        span.record("otel.status_code", "ERROR");
        span.record("error.type", e.kind_str());
    }
}

/// Resolves an edge label to its paired inverse label, if any (see
/// [`InMemoryStore`](crate::InMemoryStore)).
pub type InverseResolver = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// Map an [`sqlx::Error`] onto the store's [`Error`], collapsing the two cases the
/// store gives dedicated variants (`RowNotFound` → [`Error::NotFound`], a unique
/// violation → [`Error::AlreadyExists`]) and treating everything else as a generic
/// backend error. Dialect-independent: `is_unique_violation()` is honored by every
/// sqlx driver.
impl From<sqlx::Error> for Error {
    fn from(e: sqlx::Error) -> Self {
        match e {
            sqlx::Error::RowNotFound => Error::NotFound,
            sqlx::Error::Database(db) if db.is_unique_violation() => Error::AlreadyExists,
            other => Error::generic(other.to_string()),
        }
    }
}

/// Merge a backend's embedded schema migrations with a consumer's own into one
/// ordered ledger.
///
/// This is the shared body behind each backend's `migrator_with`. Two
/// `sqlx::migrate!` migrators would share sqlx's single hardcoded
/// `_sqlx_migrations` ledger, forcing non-overlapping version ranges and
/// `set_ignore_missing(true)` on both; merging into one migrator sidesteps that.
/// Versions across the combined set must be unique and are applied in ascending
/// order, so a consumer numbers its migrations above the backend's low range.
pub(crate) fn merge(
    base: sqlx::migrate::Migrator,
    extra: impl IntoIterator<Item = sqlx::migrate::Migration>,
) -> sqlx::migrate::Migrator {
    let mut migrations: Vec<sqlx::migrate::Migration> = base.migrations.iter().cloned().collect();
    migrations.extend(extra);
    migrations.sort_by_key(|m| m.version);
    sqlx::migrate::Migrator {
        migrations: std::borrow::Cow::Owned(migrations),
        ..sqlx::migrate::Migrator::DEFAULT
    }
}
