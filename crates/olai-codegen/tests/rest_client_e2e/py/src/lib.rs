//! The generated Python bindings, mounted under `crate::codegen` as a binding crate mounts them.

include!(concat!(env!("OUT_DIR"), "/python_mod.rs"));

/// The directory of the generated Python package `demo`.
pub const PACKAGE_DIR: &str = concat!(env!("OUT_DIR"), "/pkg");

#[cfg(test)]
mod tests;
