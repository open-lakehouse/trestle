//! Hand-written handler implementations.
//!
//! `core` holds the protocol-agnostic business logic; `greeting` and
//! `greeting_connect` are thin REST and Connect-RPC adapters that both delegate
//! into it.

pub mod core;
pub mod greeting;
pub mod greeting_connect;
