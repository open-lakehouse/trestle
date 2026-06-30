//! The vocabulary types — the model a topology is described in, with no planning,
//! rendering, or addressing logic of its own.
//!
//! A service declares the [`Role`] it fills (independent of *which* implementation
//! fills it), where it runs ([`Placement`]) relative to where a caller sits
//! ([`Vantage`]), what it offers ([`Endpoint`] with a gateway [`RouteIntent`]), and
//! the typed [`Connection`]s a resource provider vends. These types are pure data;
//! the [`plan`](crate::plan), [`render`](crate::render), and [`address`](crate::address)
//! phases are built on top of them.

pub mod connection;
pub mod endpoint;
pub mod placement;
pub mod role;

pub use connection::{Connection, ConnectionField, ConnectionTemplate, ObjectStoreCredential};
pub use endpoint::{Endpoint, Rewrite, RouteIntent, Scheme};
pub use placement::{Placement, Vantage};
pub use role::{Role, ServiceSpec};
