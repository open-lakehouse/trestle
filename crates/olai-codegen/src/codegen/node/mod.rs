//! Node.js code generation module
//!
//! Split into submodules:
//! - `caps`: NAPI capability predicates shared by both emitters
//! - `bindings`: NAPI-RS binding generation (Rust → Node.js wrapper structs)
//! - `typescript`: TypeScript client generation for idiomatic Node.js API

mod bindings;
mod caps;
pub(crate) mod typescript;

pub(crate) use bindings::{generate, main_module};
