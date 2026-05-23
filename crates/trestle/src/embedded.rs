//! Built-in templates and shared components, embedded into the binary at build time.
//!
//! The `templates/` directory at the crate root is packed into the binary via
//! [`rust_embed::RustEmbed`]. The directory layout looks like:
//!
//! ```text
//! templates/
//! ├── _components/                       # shared component library
//! │   ├── local-stack-envoy/
//! │   ├── local-stack-postgres/
//! │   └── ...
//! ├── databricks-app-rust/                # template 1
//! └── open-lakehouse-lab/                 # template 2
//! ```
//!
//! Use [`Templates::iter()`] to discover entries.
use rust_embed::Embed;
pub use rust_embed::RustEmbed;

/// The embedded `templates/` directory, baked into the binary at build time.
///
/// In release builds (and any build with the `debug-embed` feature, which we
/// always enable so that `trestle new` works without a populated source tree on
/// the user's machine), the entire `templates/` directory is materialised inside
/// the binary. In dev builds without `debug-embed`, the directory is read from
/// the filesystem on each invocation; the `iter()` and `get()` helpers behave
/// identically in both modes.
#[derive(Embed)]
#[folder = "templates"]
#[exclude = "*.DS_Store"]
#[exclude = ".gitkeep"]
pub struct Templates;

/// Prefix that marks the shared (cross-template) component library.
pub const SHARED_COMPONENTS_PREFIX: &str = "_components/";

/// Iterate over the names (top-level directory entries) of all embedded templates,
/// skipping the shared component library.
pub fn embedded_template_names() -> Vec<String> {
    let mut names = std::collections::BTreeSet::new();
    for path in <Templates as RustEmbed>::iter() {
        if path.starts_with(SHARED_COMPONENTS_PREFIX) {
            continue;
        }
        if let Some(first) = path.split('/').next() {
            names.insert(first.to_string());
        }
    }
    names.into_iter().collect()
}

/// Iterate over the names of all embedded shared components.
pub fn embedded_shared_component_names() -> Vec<String> {
    let mut names = std::collections::BTreeSet::new();
    for path in <Templates as RustEmbed>::iter() {
        if let Some(rest) = path.strip_prefix(SHARED_COMPONENTS_PREFIX) {
            if let Some(first) = rest.split('/').next() {
                names.insert(first.to_string());
            }
        }
    }
    names.into_iter().collect()
}
