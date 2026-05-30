//! Built-in templates and shared components, embedded into the binary at build time.
//!
//! The `templates/` directory at the crate root is packed into the binary via
//! [`rust_embed::RustEmbed`]. The directory layout looks like:
//!
//! ```text
//! templates/
//! ├── _base/
//! │   └── lakehouse/                     # the always-rendered base
//! ├── _apps/
//! │   └── databricks-app-rust/           # opt-in apps layered on top
//! └── _components/                       # shared component library
//!     ├── local-stack-envoy/
//!     ├── local-stack-postgres/
//!     └── ...
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

/// Prefix that marks the always-rendered base templates (e.g. the lakehouse).
pub const BASE_TEMPLATES_PREFIX: &str = "_base/";
/// Prefix that marks the opt-in app templates that can be layered on top of a base.
pub const APP_TEMPLATES_PREFIX: &str = "_apps/";
/// Prefix that marks the shared (cross-template) component library.
pub const SHARED_COMPONENTS_PREFIX: &str = "_components/";

/// Subdirectory names of all embedded base templates (e.g. `lakehouse`).
pub fn embedded_base_names() -> Vec<String> {
    immediate_children(BASE_TEMPLATES_PREFIX)
}

/// Subdirectory names of all embedded apps (e.g. `databricks-app-rust`).
pub fn embedded_app_names() -> Vec<String> {
    immediate_children(APP_TEMPLATES_PREFIX)
}

/// Subdirectory names of all embedded shared components.
pub fn embedded_shared_component_names() -> Vec<String> {
    immediate_children(SHARED_COMPONENTS_PREFIX)
}

/// Back-compat: legacy callers want every embedded "top-level template" (base or
/// app) as a flat list, by short name.
pub fn embedded_template_names() -> Vec<String> {
    let mut out = embedded_base_names();
    out.extend(embedded_app_names());
    out.sort();
    out.dedup();
    out
}

fn immediate_children(prefix: &str) -> Vec<String> {
    let mut names = std::collections::BTreeSet::new();
    for path in <Templates as RustEmbed>::iter() {
        let Some(rest) = path.strip_prefix(prefix) else {
            continue;
        };
        if let Some((first, _)) = rest.split_once('/') {
            names.insert(first.to_string());
        }
    }
    names.into_iter().collect()
}
