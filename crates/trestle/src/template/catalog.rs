//! In-memory index of shared components by name, category, and provider slot.
//!
//! The catalog is built lazily from the embedded `_components/` library (and, in
//! Phase 2+, the embedded `_apps/` library too). The wizard consults it to:
//!
//! - List components for a given category (e.g. all `category: storage` entries).
//! - Resolve a component name to its `category` / `provider_of` (for the `--with`
//!   compatibility shim, which infers the category for back-compat).
//! - Group components by category for `trestle list-components --by-category`.

use std::collections::BTreeMap;

use crate::error::Result;
use crate::template::loader::load_shared_component;
use crate::template::manifest::ComponentManifest;

/// A lightweight summary of a component, suitable for displaying in wizards or
/// list commands. Cheaper than carrying a full [`crate::template::loader::LoadedComponent`]
/// around because it skips materialising the component's `template/` tree.
#[derive(Debug, Clone)]
pub struct ComponentSummary {
    pub name: String,
    pub category: Option<String>,
    pub provider_of: Option<String>,
    pub display_name: Option<String>,
    pub summary: Option<String>,
    pub conflicts_with: Vec<String>,
}

impl ComponentSummary {
    /// Best-effort human label for the wizard. Falls back to the bare name.
    pub fn label(&self) -> &str {
        self.display_name.as_deref().unwrap_or(self.name.as_str())
    }

    fn from_manifest(m: &ComponentManifest) -> Self {
        Self {
            name: m.name.clone(),
            category: m.category.clone(),
            provider_of: m.provider_of.clone(),
            display_name: m.display_name.clone(),
            summary: m.summary.clone(),
            conflicts_with: m.conflicts_with.clone(),
        }
    }
}

/// Index of shared components, keyed by both name and category.
#[derive(Debug, Default)]
pub struct ComponentCatalog {
    by_name: BTreeMap<String, ComponentSummary>,
    by_category: BTreeMap<String, Vec<String>>,
}

impl ComponentCatalog {
    /// Build a catalog from the embedded shared component library.
    ///
    /// The materialisation work for each component is bounded (one tempdir per
    /// component), but cheap relative to a full template render: only the
    /// `template.yaml` is parsed.
    pub fn from_embedded() -> Result<Self> {
        let mut cat = Self::default();
        for name in crate::embedded::embedded_shared_component_names() {
            let loaded = load_shared_component(&name)?;
            cat.insert(ComponentSummary::from_manifest(&loaded.manifest));
        }
        Ok(cat)
    }

    fn insert(&mut self, summary: ComponentSummary) {
        if let Some(cat) = &summary.category {
            self.by_category
                .entry(cat.clone())
                .or_default()
                .push(summary.name.clone());
        }
        self.by_name.insert(summary.name.clone(), summary);
    }

    /// Look up a component by exact name.
    pub fn get(&self, name: &str) -> Option<&ComponentSummary> {
        self.by_name.get(name)
    }

    /// All components tagged with the given category id, in name-sorted order.
    pub fn components_for_category(&self, category: &str) -> Vec<&ComponentSummary> {
        match self.by_category.get(category) {
            Some(names) => names.iter().filter_map(|n| self.by_name.get(n)).collect(),
            None => Vec::new(),
        }
    }

    /// All known categories, in alphabetical order.
    pub fn categories(&self) -> Vec<&str> {
        self.by_category.keys().map(String::as_str).collect()
    }

    /// All components in name-sorted order. Useful for `list-components`.
    pub fn all(&self) -> impl Iterator<Item = &ComponentSummary> {
        self.by_name.values()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_catalog_loads_without_local_stack_components() {
        // The composed-environment components (`local-stack-*`, `databricks-emulator-env`)
        // moved out of `trestle new` into the `trestle env` engine, so the shared
        // component catalog no longer indexes them. It must still load cleanly, and
        // the compose categories it used to serve must be gone.
        let cat = ComponentCatalog::from_embedded().expect("embedded catalog loads");
        for category in [
            "gateway",
            "metadata_db",
            "storage",
            "ml",
            "catalog",
            "notebooks",
            "observability",
        ] {
            assert!(
                cat.components_for_category(category).is_empty(),
                "compose category `{category}` should no longer resolve any shared components"
            );
        }
        assert!(
            cat.all().all(|c| !c.name.starts_with("local-stack-")),
            "no `local-stack-*` component should remain in the shared catalog"
        );
    }

    #[test]
    fn every_embedded_component_is_categorised() {
        let cat = ComponentCatalog::from_embedded().expect("embedded catalog loads");
        let uncategorised: Vec<&str> = cat
            .all()
            .filter(|c| c.category.is_none())
            .map(|c| c.name.as_str())
            .collect();
        assert!(
            uncategorised.is_empty(),
            "uncategorised components: {uncategorised:?}"
        );
    }
}
