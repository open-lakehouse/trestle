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
    fn embedded_catalog_indexes_known_categories() {
        let cat = ComponentCatalog::from_embedded().expect("embedded catalog loads");

        for (category, expected_name) in [
            ("gateway", "local-stack-envoy"),
            ("metadata_db", "local-stack-postgres"),
            ("storage", "local-stack-seaweedfs"),
            ("ml", "local-stack-mlflow"),
            ("catalog", "local-stack-unity-catalog"),
            ("notebooks", "local-stack-notebooks"),
            ("observability", "local-stack-jaeger"),
            ("app_runtime", "databricks-emulator-env"),
        ] {
            let entries = cat.components_for_category(category);
            assert!(
                entries.iter().any(|c| c.name == expected_name),
                "expected `{expected_name}` under category `{category}`, got {:?}",
                entries.iter().map(|c| &c.name).collect::<Vec<_>>()
            );
        }
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
