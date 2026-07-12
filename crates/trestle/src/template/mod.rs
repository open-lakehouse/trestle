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
pub mod catalog;
pub mod hooks;
pub mod loader;
pub mod manifest;
pub mod preview;
pub mod prompt;
pub mod render;
pub mod resolve;
pub mod validate;
pub mod walk;
pub mod wizard;

pub use aggregate::{PortCollision, StackContext, aggregate_stack_context, port_collisions};
pub use catalog::{ComponentCatalog, ComponentSummary};
pub use loader::{TemplateSource, load_template};
pub use manifest::{
    Category, Component, ComponentManifest, DefaultList, LakehouseRequires, Manifest, Profile,
    Provides, VarKind, Variable, WireHelp,
};
pub use prompt::{VariableValue, collect_variables};
pub use render::{Renderer, register_filters};
pub use resolve::{ResolveInput, ResolvedComponent, ScaffoldRoot, resolve_components};
pub use walk::render_tree;
