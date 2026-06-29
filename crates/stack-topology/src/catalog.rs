//! The module [`Catalog`]: the set of modules a planner can select from, plus the
//! capability → module index that powers capability-based selection.
//!
//! A catalog is just a collection of [`Module`]s with two derived lookups: by id,
//! and by the capability each module declares it
//! [`provides`](crate::Module::provider_of). The planner resolves a selection
//! (direct module ids, or capabilities mapped through the index) against a catalog.
//!
//! Two ways to build one:
//!
//! - [`baseline_catalog`] — an inlined, pure baseline built in Rust (the common
//!   local-Lakehouse modules), always available with no I/O and no YAML dependency.
//!   This mirrors hydrofoil's static `registry()`.
//! - [`Catalog::from_manifests`] / [`Catalog::merge`] — assemble from [`Module`]
//!   values authored as YAML (a `module.yaml` per module directory) and overlay them
//!   onto the baseline. Parsing YAML is the feature-gated `catalog` concern, kept out
//!   of the pure core; on-disk *discovery* (walking a directory tree) is left to the
//!   consumer, which already owns embedding/IO.

use std::collections::BTreeMap;

use crate::module::{Module, ModuleId};

pub(crate) mod baseline;

pub use baseline::{DATA_ROOT_DEFAULT, DATA_ROOT_VAR, baseline_catalog, baseline_selection};

/// A set of modules to plan against, with id and capability indexes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Catalog {
    modules: Vec<Module>,
    /// Default provider per resource role, the deterministic tie-break the planner
    /// uses when a role has more than one provider and neither a demand pin nor a
    /// `PlanCtx` preference selects one. (Role → provider module id.)
    default_provider: BTreeMap<String, ModuleId>,
}

impl Catalog {
    /// An empty catalog.
    pub fn new() -> Self {
        Catalog::default()
    }

    /// Build a catalog from an explicit list of modules (last id wins on duplicate).
    pub fn from_modules(modules: impl IntoIterator<Item = Module>) -> Self {
        let mut cat = Catalog::new();
        for m in modules {
            cat.insert(m);
        }
        cat
    }

    /// Insert (or replace, by id) a single module.
    pub fn insert(&mut self, module: Module) {
        if let Some(slot) = self.modules.iter_mut().find(|m| m.id == module.id) {
            *slot = module;
        } else {
            self.modules.push(module);
        }
    }

    /// Overlay `other` onto `self`, replacing any module whose id matches and adding
    /// the rest. Returns `self` for chaining. This is how on-disk modules extend or
    /// override the inlined baseline.
    pub fn merge(mut self, other: Catalog) -> Self {
        for m in other.modules {
            self.insert(m);
        }
        // Overlay defaults too — a later catalog can override a role's default provider.
        for (role, provider) in other.default_provider {
            self.default_provider.insert(role, provider);
        }
        self
    }

    /// All modules in the catalog, in insertion order.
    pub fn modules(&self) -> &[Module] {
        &self.modules
    }

    /// Look up a module by id.
    pub fn get(&self, id: &ModuleId) -> Option<&Module> {
        self.modules.iter().find(|m| &m.id == id)
    }

    /// The ids of modules that declare they provide `capability` (via
    /// [`provider_of`](crate::Module::provider_of)), in catalog order.
    ///
    /// This is the capability → module index: capability-based selection maps each
    /// requested capability to its provider module(s) here, then resolves the union.
    pub fn providers_of(&self, capability: &str) -> Vec<&ModuleId> {
        self.modules
            .iter()
            .filter(|m| m.provider_of.as_deref() == Some(capability))
            .map(|m| &m.id)
            .collect()
    }

    /// Every capability declared by some module, mapped to the providing module ids.
    pub fn capability_index(&self) -> BTreeMap<String, Vec<ModuleId>> {
        let mut index: BTreeMap<String, Vec<ModuleId>> = BTreeMap::new();
        for m in &self.modules {
            if let Some(cap) = &m.provider_of {
                index.entry(cap.clone()).or_default().push(m.id.clone());
            }
        }
        index
    }

    /// The ids of modules that provision `resource_kind` (declare it under
    /// [`Provides::resource_kinds`](crate::Provides::resource_kinds)), in catalog
    /// order. This is the resource index the planner uses to auto-provision a provider
    /// for a [`ResourceDemand`](crate::ResourceDemand).
    pub fn providers_for(&self, resource_kind: &str) -> Vec<&ModuleId> {
        self.modules
            .iter()
            .filter(|m| m.provides.resource_kinds.contains_key(resource_kind))
            .map(|m| &m.id)
            .collect()
    }

    /// Declare the default provider for a resource role (builder-style; returns `self`).
    /// The planner uses this as the final tie-break when a role has multiple providers
    /// and no pin/preference selects one.
    pub fn with_default_provider(
        mut self,
        role: impl Into<String>,
        provider: impl Into<ModuleId>,
    ) -> Self {
        self.default_provider.insert(role.into(), provider.into());
        self
    }

    /// The declared default provider for `role`, if any.
    pub fn default_provider_for(&self, role: &str) -> Option<&ModuleId> {
        self.default_provider.get(role)
    }

    /// The single provider for `resource_kind`, if exactly one module provisions it.
    ///
    /// Returns `Ok(None)` when no module provides the kind, `Ok(Some(id))` for exactly
    /// one, and `Err(ids)` (all candidates, sorted) when more than one does — the
    /// planner turns these into `UnsatisfiedDemand` / `AmbiguousProvider`.
    pub fn unique_provider_for(
        &self,
        resource_kind: &str,
    ) -> Result<Option<&ModuleId>, Vec<ModuleId>> {
        let providers = self.providers_for(resource_kind);
        match providers.len() {
            0 => Ok(None),
            1 => Ok(Some(providers[0])),
            _ => {
                let mut ids: Vec<ModuleId> = providers.into_iter().cloned().collect();
                ids.sort();
                Err(ids)
            }
        }
    }

    /// Parse and assemble a catalog from a sequence of module-manifest YAML strings.
    ///
    /// Each element is the text of one module manifest (a `module.yaml`), a single
    /// [`Module`] document. The pure core can construct [`baseline_catalog`] without
    /// this; YAML parsing is gated behind the `catalog` feature so the core stays
    /// free of `serde_yaml`.
    #[cfg(feature = "catalog")]
    pub fn from_manifests<'a>(
        manifests: impl IntoIterator<Item = &'a str>,
    ) -> Result<Catalog, serde_yaml::Error> {
        let mut cat = Catalog::new();
        for text in manifests {
            cat.insert(serde_yaml::from_str(text)?);
        }
        Ok(cat)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_has_the_default_lakehouse_modules() {
        let cat = baseline_catalog();
        for id in ["envoy", "postgres", "seaweedfs", "mlflow", "unity-catalog"] {
            assert!(cat.get(&id.into()).is_some(), "baseline missing {id}");
        }
    }

    #[test]
    fn capability_index_maps_provider_of() {
        let cat = baseline_catalog();
        assert_eq!(
            cat.providers_of("experiment_tracking"),
            vec![&ModuleId::from("mlflow")]
        );
        assert_eq!(
            cat.providers_of("data_catalog"),
            vec![&ModuleId::from("unity-catalog")]
        );
    }

    #[test]
    fn merge_overlays_by_id_without_duplicating() {
        let mut overlay = baseline_catalog().get(&"envoy".into()).cloned().unwrap();
        overlay.summary = Some("overridden".into());
        let cat = baseline_catalog().merge(Catalog::from_modules([overlay]));
        assert_eq!(
            cat.get(&"envoy".into()).unwrap().summary.as_deref(),
            Some("overridden")
        );
        assert_eq!(
            cat.modules()
                .iter()
                .filter(|m| m.id.as_str() == "envoy")
                .count(),
            1
        );
    }
}
