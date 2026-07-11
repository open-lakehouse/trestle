//! Default store backends shipped with `olai-store`.
//!
//! - [`mem`] — [`InMemoryStore`](mem::InMemoryStore), an always-available,
//!   dependency-free reference backend.
//!
//! A persistent, sqlx-backed `SqlStore` lives behind the `sqlite` / `postgres`
//! features (see the `sql` module when those features are enabled).

pub mod mem;

#[cfg(feature = "sqlite")]
pub mod sql;
