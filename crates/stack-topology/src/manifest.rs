//! The persisted, deterministic record of a user's environment choices: [`EnvManifest`].
//!
//! An environment is fully determined by two inputs to [`Catalog::plan`](crate::Catalog::plan):
//! a [`Selection`] (which modules + knob overrides) and a [`PlanCtx`] (the environment-level
//! facts the model can't derive — gateway ports, `data_root`, provider preferences, the app
//! upstream). An [`EnvManifest`] bundles both and (de)serializes to **TOML**, so a consumer can
//! save a user's choices and re-create the *same* environment later.
//!
//! # Stability across edits
//!
//! The manifest is also the **edit surface**: load it, [`add_module`](EnvManifest::add_module) /
//! [`remove_module`](EnvManifest::remove_module), and re-[`plan`](EnvManifest::plan). The
//! **shared gateway listener is stable by construction** — its host port comes from
//! [`PlanCtx::gateway_host_port`] (persisted here, never re-derived), and every shared route's
//! prefix / rewrite / cluster is derived purely from the owning module's own static data, so a
//! surviving module keeps its exact mapping no matter what else is added or removed. This is the
//! guarantee host applications depend on: the gateway port and the route to a given service do
//! not move when the selection is edited.
//!
//! Two things *may* move on an edit, by design:
//!
//! - **Dedicated/secondary listener ports** ([`UiFixed`](crate::RouteIntent::UiFixed) /
//!   [`Gatewayed`](crate::RouteIntent::Gatewayed)) — allocated sequentially in graph order, so
//!   adding a module with such an endpoint can shift later ones. Use
//!   [`pin_dedicated_ports`] to hold them stable best-effort (it feeds
//!   [`PlanCtx::dedicated_listener_ports`] on the next plan).
//! - **Provider selection** for an *unpinned* resource demand — set
//!   [`PlanCtx::provider_preference`] (persisted here) to pin which implementation satisfies a
//!   role, and this is stable too.
//!
//! # Purity
//!
//! [`to_toml`](EnvManifest::to_toml) / [`from_toml`](EnvManifest::from_toml) are pure and
//! WASM-clean (they produce / consume a `String`). Only [`read_from`](EnvManifest::read_from) /
//! [`write_to`](EnvManifest::write_to) touch the filesystem, and they sit behind the `std-io`
//! feature — mirroring [`MaterializedOutput::write_to`](crate::MaterializedOutput::write_to).

use serde::{Deserialize, Serialize};

use crate::catalog::Catalog;
use crate::catalog::module::ModuleId;
use crate::plan::{Plan, PlanCtx, PlanError, Selection};

/// What can go wrong (de)serializing an [`EnvManifest`].
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    /// Serializing the manifest to TOML failed.
    #[error("serializing the environment manifest to TOML failed: {0}")]
    Serialize(#[from] toml::ser::Error),
    /// Parsing the manifest from TOML failed (malformed file or unknown shape).
    #[error("parsing the environment manifest from TOML failed: {0}")]
    Deserialize(#[from] toml::de::Error),
    /// Reading or writing the manifest file failed (only the `std-io` helpers).
    #[cfg(feature = "std-io")]
    #[error("reading/writing the environment manifest file failed: {0}")]
    Io(#[from] std::io::Error),
}

/// A persisted, deterministic record of a user's environment choices: the module + knob
/// [`Selection`] and the environment-level [`PlanCtx`]. Re-[`plan`](Self::plan)ning from the same
/// manifest yields the same [`Plan`] — in particular the same gateway host port, which host
/// applications depend on. See the [module docs](crate::manifest) for the stability contract.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvManifest {
    /// The manifest schema version, so older files stay readable as the shape evolves.
    /// Omitted files default to [`current_version`](Self::current_version).
    #[serde(default = "EnvManifest::current_version")]
    pub version: u32,
    /// Which modules / capabilities are selected and their knob overrides.
    #[serde(default)]
    pub selection: Selection,
    /// The environment-level plan context — gateway ports, `data_root`, provider preferences,
    /// app upstream. A persisted non-default `gateway_host_port` is authoritative on reload, so
    /// the host-facing port stays put across re-plans.
    #[serde(default)]
    pub context: PlanCtx,
}

impl EnvManifest {
    /// The current manifest schema version.
    pub fn current_version() -> u32 {
        1
    }

    /// A manifest for `selection` under `context`, at the current schema version.
    pub fn new(selection: Selection, context: PlanCtx) -> Self {
        EnvManifest {
            version: Self::current_version(),
            selection,
            context,
        }
    }

    /// Serialize this manifest to a TOML string. Pure and WASM-clean.
    pub fn to_toml(&self) -> Result<String, ManifestError> {
        Ok(toml::to_string_pretty(self)?)
    }

    /// Parse a manifest from a TOML string. Pure and WASM-clean. Fields the file omits fall back
    /// to their defaults (e.g. an absent `[context]` uses [`PlanCtx::default`]).
    pub fn from_toml(s: &str) -> Result<Self, ManifestError> {
        Ok(toml::from_str(s)?)
    }

    /// Resolve this manifest into a [`Plan`] — the single "manifest → environment" entry point.
    /// Wraps [`Catalog::plan`](crate::Catalog::plan) with the manifest's own selection + context.
    pub fn plan(&self, catalog: &Catalog) -> Result<Plan, PlanError> {
        catalog.plan(&self.selection, &self.context)
    }

    /// Select `id`, if not already selected. Idempotent.
    pub fn add_module(&mut self, id: impl Into<ModuleId>) {
        let id = id.into();
        if !self.selection.modules.contains(&id) {
            self.selection.modules.push(id);
        }
    }

    /// Deselect `id` and drop any knob overrides recorded for it, keeping the manifest tidy.
    pub fn remove_module(&mut self, id: &ModuleId) {
        self.selection.modules.retain(|m| m != id);
        self.selection.knob_overrides.remove(id);
    }

    /// Select capability `cap`, if not already selected. Idempotent.
    pub fn add_capability(&mut self, cap: impl Into<String>) {
        let cap = cap.into();
        if !self.selection.capabilities.contains(&cap) {
            self.selection.capabilities.push(cap);
        }
    }

    /// Deselect capability `cap`.
    pub fn remove_capability(&mut self, cap: &str) {
        self.selection.capabilities.retain(|c| c != cap);
    }

    /// Set a knob override for `module`: `key` → `value`, replacing any previous value.
    pub fn set_knob(
        &mut self,
        module: impl Into<ModuleId>,
        key: impl Into<String>,
        value: impl Into<String>,
    ) {
        self.selection
            .knob_overrides
            .entry(module.into())
            .or_default()
            .insert(key.into(), value.into());
    }
}

/// Capture the dedicated-listener host ports a plan assigned, as the explicit
/// [`dedicated_listener_ports`](PlanCtx::dedicated_listener_ports) list to feed a later re-plan so
/// surviving dedicated endpoints keep their ports.
///
/// The ports are returned in the plan's listener order (graph order), so re-feeding them — which
/// the planner consumes positionally, also in graph order — keeps a surviving dedicated endpoint
/// on its previous port. This is **best-effort**: large topology changes that reorder the graph or
/// insert/remove dedicated endpoints can still shift the positional alignment. The shared listener
/// is stable without this; pinning here is purely for the secondary listeners.
pub fn pin_dedicated_ports(plan: &Plan) -> Vec<u16> {
    // Every listener that isn't the shared one (its host port is the gateway's host port) is a
    // dedicated listener, already in graph order on the plan.
    let shared = plan.gateway_host_port();
    plan.gateway
        .listeners
        .iter()
        .filter(|l| l.host_port != shared)
        .map(|l| l.host_port)
        .collect()
}

#[cfg(feature = "std-io")]
impl EnvManifest {
    /// Read and parse a manifest from `path`. Behind `std-io` (the only filesystem touch).
    pub fn read_from(path: &std::path::Path) -> Result<Self, ManifestError> {
        Self::from_toml(&std::fs::read_to_string(path)?)
    }

    /// Serialize this manifest and write it to `path`, creating parent directories as needed.
    /// Behind `std-io` (the only filesystem touch).
    pub fn write_to(&self, path: &std::path::Path) -> Result<(), ManifestError> {
        let toml = self.to_toml()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, toml)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::baseline::baseline_catalog;

    fn sample_manifest() -> EnvManifest {
        EnvManifest::new(
            Selection::modules(["envoy", "postgres", "seaweedfs", "unity-catalog", "mlflow"]),
            PlanCtx::default(),
        )
    }

    #[test]
    fn toml_round_trips_exactly() {
        let manifest = sample_manifest();
        let toml = manifest.to_toml().unwrap();
        let back = EnvManifest::from_toml(&toml).unwrap();
        assert_eq!(manifest, back);
    }

    #[test]
    fn partial_toml_fills_context_defaults() {
        // Only a selection; no [context] table at all.
        let toml = r#"
            [selection]
            modules = ["envoy", "postgres", "mlflow"]
        "#;
        let manifest = EnvManifest::from_toml(toml).unwrap();
        assert_eq!(manifest.version, EnvManifest::current_version());
        assert_eq!(manifest.context.gateway_host_port, 9080);
        assert_eq!(manifest.context.env_name, "lakehouse");
    }

    #[test]
    fn saved_gateway_host_port_wins_over_default() {
        let mut manifest = sample_manifest();
        manifest.context.gateway_host_port = 9090;
        let toml = manifest.to_toml().unwrap();
        let reloaded = EnvManifest::from_toml(&toml).unwrap();
        let plan = reloaded.plan(&baseline_catalog()).unwrap();
        assert_eq!(plan.gateway_host_port(), 9090);
        // The shared listener (the one published on the host port) reflects it.
        assert!(
            plan.gateway.listeners.iter().any(|l| l.host_port == 9090),
            "shared listener should publish the saved host port"
        );
    }

    #[test]
    fn plan_is_deterministic_across_replans() {
        let manifest = sample_manifest();
        let catalog = baseline_catalog();
        let a = crate::render_all(&manifest.plan(&catalog).unwrap());
        let b = crate::render_all(&manifest.plan(&catalog).unwrap());
        assert_eq!(a.compose, b.compose);
        assert_eq!(a.envoy, b.envoy);
        assert_eq!(a.env, b.env);
    }

    #[test]
    fn editing_selection_keeps_shared_listener_stable() {
        let catalog = baseline_catalog();
        let mut manifest = sample_manifest();

        // Snapshot the shared listener (host port + its routes) before the edit.
        let before = manifest.plan(&catalog).unwrap();
        let shared_port = before.gateway_host_port();
        let shared_routes_before: Vec<_> = before
            .gateway
            .listeners
            .iter()
            .find(|l| l.host_port == shared_port)
            .unwrap()
            .routes
            .clone();

        // Add a module that contributes its own surface routes.
        manifest.add_module("headwaters");
        let after = manifest.plan(&catalog).unwrap();
        assert_eq!(
            after.gateway_host_port(),
            shared_port,
            "host port must not move"
        );
        let shared_after = after
            .gateway
            .listeners
            .iter()
            .find(|l| l.host_port == shared_port)
            .unwrap();
        // Every prior route survives with the same cluster + rewrite.
        for route in &shared_routes_before {
            assert!(
                shared_after.routes.contains(route),
                "shared route {route:?} should survive the add unchanged"
            );
        }

        // Removing the module restores the exact original shared listener.
        manifest.remove_module(&ModuleId::from("headwaters"));
        let restored = manifest.plan(&catalog).unwrap();
        let shared_restored = restored
            .gateway
            .listeners
            .iter()
            .find(|l| l.host_port == shared_port)
            .unwrap();
        assert_eq!(shared_restored.routes, shared_routes_before);
    }

    #[test]
    fn set_and_remove_knob_and_capability() {
        let mut m = EnvManifest::new(Selection::default(), PlanCtx::default());
        m.add_module("envoy");
        m.add_module("envoy"); // idempotent
        assert_eq!(m.selection.modules.len(), 1);
        m.set_knob("envoy", "ENVOY_AUTH", "true");
        assert_eq!(
            m.selection.knob_overrides[&ModuleId::from("envoy")].get("ENVOY_AUTH"),
            Some(&"true".to_string())
        );
        m.remove_module(&ModuleId::from("envoy"));
        assert!(m.selection.modules.is_empty());
        assert!(
            m.selection.knob_overrides.is_empty(),
            "knobs dropped with module"
        );

        m.add_capability("experiment_tracking");
        m.add_capability("experiment_tracking"); // idempotent
        assert_eq!(m.selection.capabilities.len(), 1);
        m.remove_capability("experiment_tracking");
        assert!(m.selection.capabilities.is_empty());
    }
}

#[cfg(all(test, feature = "std-io"))]
mod std_io_tests {
    use super::*;
    use crate::plan::Selection;

    #[test]
    fn write_then_read_round_trips() {
        let manifest = EnvManifest::new(
            Selection::modules(["envoy", "postgres", "mlflow"]),
            PlanCtx::default(),
        );
        let dir =
            std::env::temp_dir().join(format!("stack-topology-manifest-{}", std::process::id()));
        let path = dir.join("env.toml");
        let _ = std::fs::remove_dir_all(&dir);
        manifest.write_to(&path).unwrap();
        let back = EnvManifest::read_from(&path).unwrap();
        assert_eq!(manifest, back);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
