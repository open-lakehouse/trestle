//! The generated models (`crate::models`) and REST client (`crate::codegen`) of
//! `proto/demo/v1/svc.proto`, mounted as a client crate mounts them.

// The generated code refers to this crate by name.
extern crate self as rest_client_e2e;

pub mod models {
    include!(concat!(env!("OUT_DIR"), "/models_mod.rs"));
    pub use _gen::*;
}

pub mod codegen {
    include!(concat!(env!("OUT_DIR"), "/client/mod.rs"));
}

// The binding crate imports the clients and the error module from the crate root.
pub use codegen::*;

/// Context type of the (unused) server config.
pub struct Context;

#[cfg(test)]
mod tests;
