//! Default store backends shipped with `olai-store`.
//!
//! - [`mem`] — [`InMemoryStore`](mem::InMemoryStore), an always-available,
//!   dependency-free reference backend.
//!
//! Persistent, sqlx-backed backends live behind features:
//! - [`sqlite`] (feature `sqlite`) — [`SqlStore`](sqlite::SqlStore), a SQLite backend.
//! - [`postgres`] (feature `postgres`) — [`PgStore`](postgres::PgStore), a native
//!   Postgres backend (jsonb + GIN filter pushdown, `RETURNING`-based CAS, ICU
//!   case-insensitive names).
//!
//! Both delegate to the same DB-agnostic trait layer in [`crate::store`] and pass
//! the shared [`conformance`](crate::conformance) battery.

pub mod mem;

// Glue shared by every sqlx backend (error mapping, the inverse-edge resolver,
// the migrator-merge helper, and the trait-delegation macros). Compiled whenever
// at least one sqlx backend is enabled so it is defined exactly once even when
// both `sqlite` and `postgres` are on.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) mod sql_common;

#[cfg(feature = "sqlite")]
pub mod sqlite;

#[cfg(feature = "postgres")]
pub mod postgres;

// Back-compat: the SQLite backend used to live at `backend::sql`. Keep the old
// path pointing at it so existing `backend::sql::…` references still resolve.
#[cfg(feature = "sqlite")]
pub use sqlite as sql;
