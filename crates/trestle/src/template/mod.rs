//! Template engine: manifest parsing, component resolution, file rendering, and
//! post-init hooks.
//!
//! ## High-level flow
//!
//! ```text
//! load TemplateSource ──► parse manifest
//!         │
//!         ▼
//! prompt for variables (manifest.variables)
//!         │
//!         ▼
//! resolve active components (`when:` + `depends_on`)
//!         │
//!         ▼
//! render parent template tree + each active component tree ──► output dir
//!         │
//!         ▼
//! run post_init hooks
//! ```

pub mod hooks;
pub mod loader;
pub mod manifest;
pub mod prompt;
pub mod render;
pub mod resolve;
pub mod walk;
pub mod wizard;

pub use loader::{TemplateSource, load_template};
pub use manifest::{
    Category, Component, ComponentManifest, DefaultList, Manifest, VarKind, Variable,
};
pub use prompt::{VariableValue, collect_variables};
pub use render::{Renderer, register_filters};
pub use resolve::{ResolveInput, ResolvedComponent, ScaffoldRoot, resolve_components};
pub use walk::render_tree;
