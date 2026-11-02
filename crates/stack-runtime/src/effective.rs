//! [`EffectiveEnv`]: an instance's environment, from every layer, with provenance.

use std::collections::BTreeMap;
use std::fmt;

use olai_stack_state::{VarSet, VarValue};
use serde::Serialize;

use crate::secrets::{SecretError, SecretProviders, SecretValue};
use crate::source::Overlay;

/// Where a variable's value comes from, lowest precedence first. A higher layer overrides a
/// lower one, except over [`Placement`](Layer::Placement), whose keys nothing overrides.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(tag = "layer", content = "name", rename_all = "snake_case")]
pub enum Layer {
    /// The catalog's defaults (e.g. the in-stack development credentials).
    CatalogDefaults,
    /// What the revision's manifest plans to (the plan's injected environment).
    RevisionManifest,
    /// The instance's placement: reserved keys such as the compose project.
    Placement,
    /// How a client reaches the stack (the plan's client environment).
    ClientEnv,
    /// The workspace's vars.
    WorkspaceVars,
    /// The environment's vars.
    EnvironmentVars,
    /// The revision's vars.
    RevisionVars,
    /// A source's overlay, by source name.
    Source(String),
    /// The instance's overrides.
    InstanceOverrides,
}

/// One variable's value: plain, or a secret.
#[derive(Clone, PartialEq, Eq)]
pub enum EnvValue {
    /// A plain value.
    Plain(String),
    /// A secret's value, masked wherever shown.
    Secret(SecretValue),
}

impl EnvValue {
    /// The value.
    pub fn expose(&self) -> &str {
        match self {
            EnvValue::Plain(v) => v,
            EnvValue::Secret(s) => s.expose(),
        }
    }

    /// Whether it is a secret.
    pub fn is_secret(&self) -> bool {
        matches!(self, EnvValue::Secret(_))
    }
}

impl fmt::Debug for EnvValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EnvValue::Plain(v) => f.debug_tuple("Plain").field(v).finish(),
            EnvValue::Secret(_) => f.write_str("Secret(<redacted>)"),
        }
    }
}

/// A variable as resolved: its value, the layer it came from, and the lower layers it
/// overrode (or, for a placement key, the layers that tried and were ignored).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    /// The value.
    pub value: EnvValue,
    /// The layer that set it.
    pub provenance: Layer,
    /// The other layers that set it, lowest first.
    pub shadowed: Vec<Layer>,
}

/// What one layer sets, before secrets are resolved.
#[derive(Clone, Debug)]
pub enum LayerValue {
    /// A plain value.
    Plain(String),
    /// A value to resolve with a [`SecretProviders`].
    SecretRef(olai_stack_state::SecretRef),
    /// A secret value already resolved (a source's credential).
    Secret(SecretValue),
}

/// The layers an [`EffectiveEnv`] is resolved from, each a map of variables. Build it with
/// the `with_*` methods in any order; resolution applies [`Layer`] order.
#[derive(Clone, Debug, Default)]
pub struct EnvLayers {
    layers: BTreeMap<Layer, BTreeMap<String, LayerValue>>,
    /// The order sources apply in (their [`Layer::Source`] keys sort by name otherwise).
    source_order: Vec<String>,
}

fn plain(map: &BTreeMap<String, String>) -> BTreeMap<String, LayerValue> {
    map.iter()
        .map(|(k, v)| (k.clone(), LayerValue::Plain(v.clone())))
        .collect()
}

fn var_set(set: &VarSet) -> BTreeMap<String, LayerValue> {
    set.0
        .iter()
        .map(|(k, v)| {
            let v = match v {
                VarValue::Literal(l) => LayerValue::Plain(l.clone()),
                VarValue::Secret(r) => LayerValue::SecretRef(r.clone()),
            };
            (k.clone(), v)
        })
        .collect()
}

impl EnvLayers {
    /// No layers.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a plain layer.
    pub fn with_plain(mut self, layer: Layer, vars: &BTreeMap<String, String>) -> Self {
        self.layers.entry(layer).or_default().extend(plain(vars));
        self
    }

    /// Set a layer of secret values (e.g. the catalog's default credentials).
    pub fn with_secrets(mut self, layer: Layer, vars: &BTreeMap<String, String>) -> Self {
        self.layers.entry(layer).or_default().extend(
            vars.iter()
                .map(|(k, v)| (k.clone(), LayerValue::Secret(SecretValue::new(v.clone())))),
        );
        self
    }

    /// Set a layer from a store [`VarSet`] (literals and secret references).
    pub fn with_vars(mut self, layer: Layer, vars: &VarSet) -> Self {
        self.layers.entry(layer).or_default().extend(var_set(vars));
        self
    }

    /// Add a source's overlay, after the sources added before it.
    pub fn with_overlay(mut self, source: &str, overlay: &Overlay) -> Self {
        let mut vars = plain(&overlay.vars);
        vars.extend(plain(&overlay.endpoints));
        vars.extend(
            overlay
                .credentials
                .iter()
                .map(|(k, v)| (k.clone(), LayerValue::Secret(v.clone()))),
        );
        self.layers
            .entry(Layer::Source(source.to_owned()))
            .or_default()
            .extend(vars);
        if !self.source_order.iter().any(|s| s == source) {
            self.source_order.push(source.to_owned());
        }
        self
    }

    fn ordered(&self) -> Vec<(&Layer, &BTreeMap<String, LayerValue>)> {
        let rank = |layer: &Layer| match layer {
            Layer::Source(name) => (
                7,
                self.source_order
                    .iter()
                    .position(|s| s == name)
                    .unwrap_or(usize::MAX),
            ),
            Layer::InstanceOverrides => (8, 0),
            other => (
                [
                    Layer::CatalogDefaults,
                    Layer::RevisionManifest,
                    Layer::Placement,
                    Layer::ClientEnv,
                    Layer::WorkspaceVars,
                    Layer::EnvironmentVars,
                    Layer::RevisionVars,
                ]
                .iter()
                .position(|l| l == other)
                .unwrap_or(0),
                0,
            ),
        };
        let mut layers: Vec<_> = self.layers.iter().collect();
        layers.sort_by_key(|(layer, _)| rank(layer));
        layers
    }

    /// Resolve every variable: the highest layer setting it wins (a placement key can't be
    /// overridden), then secret references are resolved with `providers`. Any reference that
    /// can't be resolved is an error; nothing falls back. A key some layer holds as a secret
    /// stays masked even where a higher layer sets it as a plain literal.
    pub fn resolve(&self, providers: &SecretProviders) -> Result<EffectiveEnv, SecretError> {
        let mut winners: BTreeMap<String, (Layer, &LayerValue, Vec<Layer>)> = BTreeMap::new();
        let mut secret_keys: std::collections::BTreeSet<&str> = Default::default();
        for (layer, vars) in self.ordered() {
            for (key, value) in vars {
                if !matches!(value, LayerValue::Plain(_)) {
                    secret_keys.insert(key);
                }
                match winners.get_mut(key) {
                    Some((winner, _, shadowed)) if *winner == Layer::Placement => {
                        shadowed.push(layer.clone());
                    }
                    Some((winner, current, shadowed)) => {
                        shadowed.push(std::mem::replace(winner, layer.clone()));
                        *current = value;
                    }
                    None => {
                        winners.insert(key.clone(), (layer.clone(), value, Vec::new()));
                    }
                }
            }
        }
        // Secrets last, once every layer has had its say.
        let mut vars = BTreeMap::new();
        for (key, (provenance, value, shadowed)) in winners {
            let value = match value {
                LayerValue::Plain(v) if secret_keys.contains(key.as_str()) => {
                    EnvValue::Secret(SecretValue::new(v.clone()))
                }
                LayerValue::Plain(v) => EnvValue::Plain(v.clone()),
                LayerValue::Secret(s) => EnvValue::Secret(s.clone()),
                LayerValue::SecretRef(r) => EnvValue::Secret(providers.resolve(r)?),
            };
            vars.insert(
                key,
                Resolved {
                    value,
                    provenance,
                    shadowed,
                },
            );
        }
        Ok(EffectiveEnv { vars })
    }
}

/// An instance's resolved environment: every variable with its value and provenance. The
/// same resolution feeds what a UI shows ([`masked`](Self::masked)) and what the instance
/// launches with ([`launch_vars`](Self::launch_vars)).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EffectiveEnv {
    vars: BTreeMap<String, Resolved>,
}

/// One row of [`EffectiveEnv::masked`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MaskedVar {
    /// The value, or `"••••••"` for a secret.
    pub value: String,
    /// Whether it is a secret.
    pub secret: bool,
    /// The layer that set it.
    pub provenance: Layer,
    /// The other layers that set it, lowest first.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub shadowed: Vec<Layer>,
}

/// What a masked secret shows as.
pub const MASK: &str = "••••••";

impl EffectiveEnv {
    /// Every variable, by name.
    pub fn vars(&self) -> &BTreeMap<String, Resolved> {
        &self.vars
    }

    /// The value of `key`.
    pub fn get(&self, key: &str) -> Option<&EnvValue> {
        self.vars.get(key).map(|r| &r.value)
    }

    /// Every variable with secrets masked, for showing.
    pub fn masked(&self) -> BTreeMap<String, MaskedVar> {
        self.vars
            .iter()
            .map(|(k, r)| {
                let secret = r.value.is_secret();
                let value = if secret {
                    MASK.to_owned()
                } else {
                    r.value.expose().to_owned()
                };
                (
                    k.clone(),
                    MaskedVar {
                        value,
                        secret,
                        provenance: r.provenance.clone(),
                        shadowed: r.shadowed.clone(),
                    },
                )
            })
            .collect()
    }

    /// Every variable's value, for the launch environment.
    pub fn launch_vars(&self) -> BTreeMap<String, String> {
        self.vars
            .iter()
            .map(|(k, r)| (k.clone(), r.value.expose().to_owned()))
            .collect()
    }
}
