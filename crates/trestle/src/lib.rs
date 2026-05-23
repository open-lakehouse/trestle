//! Trestle — a unified CLI for proto-driven code generation and full-project scaffolding.
//!
//! Trestle composes two responsibilities:
//!
//! 1. **Code generation** from protobuf descriptors (the original `proto-gen` toolchain
//!    implemented in [`olai_codegen`]).
//! 2. **Project scaffolding** from versioned template trees with composable platform-stack
//!    components ("trestle new").
//!
//! The CLI binary is `trestle`. See [`cli`] for the command surface and [`template`] for
//! the templating engine internals.

pub mod cli;
pub mod embedded;
pub mod error;
pub mod template;

pub use error::{Error, Result};
