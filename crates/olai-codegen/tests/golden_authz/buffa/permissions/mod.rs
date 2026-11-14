// @generated — do not edit by hand.
pub use handler::PermissionHandler;

mod authz;
mod handler;
#[cfg(feature = "axum")]
pub mod server;
