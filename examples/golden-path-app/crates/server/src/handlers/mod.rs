//! Hand-written handler implementations.
//!
//! `core` holds the protocol-agnostic business logic; `greeting` is the REST
//! adapter implementing the generated `GreetingHandler` trait, and
//! `greeting_connect` is the Connect-RPC adapter. Both adapters delegate into
//! the shared `core`.

pub mod core;
pub mod greeting;
pub mod greeting_connect;
