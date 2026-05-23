//! Template engine: manifest parsing, component resolution, context aggregation,
//! file rendering, and post-init hooks.
//!
//! ## High-level flow
//!
//! ```text
//! load TemplateSource ──► parse manifest ──► resolve active components (profile + deps)
//!                                                         │
//!                                                         ▼
//! prompt for variables ◄── manifest.variables ──► aggregate provides → `stack` context
//!         │
//!         ▼
//! render parent template tree + each active component tree ──► output dir
//!         │
//!         ▼
//! run post_init hooks
//! ```

pub mod aggregate;
pub mod hooks;
pub mod loader;
pub mod manifest;
pub mod prompt;
pub mod render;
pub mod resolve;
pub mod walk;

pub use aggregate::{StackContext, aggregate_stack_context};
pub use loader::{TemplateSource, load_template};
pub use manifest::{Component, ComponentManifest, Manifest, Profile, Provides, VarKind, Variable};
pub use prompt::{VariableValue, collect_variables};
pub use render::{Renderer, register_filters};
pub use resolve::resolve_components;
pub use walk::render_tree;
