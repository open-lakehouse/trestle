//! Public glue between Axum and generated handlers.

pub mod context;
pub mod error;

pub use context::RequestContext;
pub use error::{Error, Result};