//! Default store backends shipped with `olai-store`.
//!
//! - [`mem`] — [`InMemoryStore`](mem::InMemoryStore), an always-available,
//!   dependency-free reference backend.
//!
//! A persistent, sqlx-backed `SqlStore` lives behind the `sqlite` / `postgres`
//! features (see the `sql` module when those features are enabled).

pub mod mem;
// `sql` (feature-gated `SqlStore`) is added in a later step.
