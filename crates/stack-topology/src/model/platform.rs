//! Platform features: environment-wide capabilities (identity, policy, observability) that a
//! selection turns on once, and that every capable service then integrates with.
//!
//! A feature is filled by one provider module (the `auth`, `policy` or `tracing` role). Other
//! modules declare what they *can* integrate with ([`IntegrationSpec`]) and over which
//! [protocols](protocol), and a provider declares the protocols it serves
//! ([`ProtocolOffer`]). The planner turns an integration on wherever the two meet, unless the
//! selection opts the module out (see [`Selection::integrations`](crate::Selection::integrations)).

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::role::Role;
use crate::catalog::module::ModuleId;

/// An environment-wide capability services integrate with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlatformFeature {
    /// Who the caller is: sign-in, and the identity it asserts to the services behind it.
    Identity,
    /// What the caller may do: an authorization policy engine.
    Policy,
    /// Where services send their telemetry: a tracing backend.
    Observability,
}

impl PlatformFeature {
    /// Every feature, in the order a UI lists them.
    pub const ALL: [PlatformFeature; 3] = [Self::Identity, Self::Policy, Self::Observability];

    /// The feature's identifier (`identity`, `policy`, `observability`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::Policy => "policy",
            Self::Observability => "observability",
        }
    }

    /// The role its providers fill.
    pub fn role(self) -> &'static str {
        match self {
            Self::Identity => Role::AUTH,
            Self::Policy => Role::POLICY,
            Self::Observability => Role::TRACING,
        }
    }

    /// The feature whose providers fill `role`, if any.
    pub fn of_role(role: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.role() == role)
    }
}

impl std::fmt::Display for PlatformFeature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Which provider fills a selected feature.
///
/// Persisted as `true` for the catalog's default provider, or as the provider's module id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FeatureChoice {
    /// The catalog's first provider of the feature's role.
    Default,
    /// This provider.
    Provider(ModuleId),
}

impl FeatureChoice {
    /// The provider it names, if it names one.
    pub fn provider(&self) -> Option<&ModuleId> {
        match self {
            Self::Default => None,
            Self::Provider(id) => Some(id),
        }
    }
}

/// The platform features a selection turns on, each with its provider (see the
/// [module docs](self)). A feature left unset is off.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformSelection {
    /// Sign-in, enforced by the gateway. Needs the gateway.
    #[serde(default, with = "choice", skip_serializing_if = "Option::is_none")]
    pub identity: Option<FeatureChoice>,
    /// An authorization policy engine. Needs [`identity`](Self::identity): a policy decides on
    /// the identity sign-in asserts.
    #[serde(default, with = "choice", skip_serializing_if = "Option::is_none")]
    pub policy: Option<FeatureChoice>,
    /// A tracing backend services export their traces to.
    #[serde(default, with = "choice", skip_serializing_if = "Option::is_none")]
    pub observability: Option<FeatureChoice>,
}

impl PlatformSelection {
    /// Whether no feature is on.
    pub fn is_empty(&self) -> bool {
        PlatformFeature::ALL.iter().all(|f| self.get(*f).is_none())
    }

    /// The choice for `feature`, if it is on.
    pub fn get(&self, feature: PlatformFeature) -> Option<&FeatureChoice> {
        match feature {
            PlatformFeature::Identity => self.identity.as_ref(),
            PlatformFeature::Policy => self.policy.as_ref(),
            PlatformFeature::Observability => self.observability.as_ref(),
        }
    }

    /// Set (or, with `None`, turn off) `feature`.
    pub fn set(&mut self, feature: PlatformFeature, choice: Option<FeatureChoice>) {
        let slot = match feature {
            PlatformFeature::Identity => &mut self.identity,
            PlatformFeature::Policy => &mut self.policy,
            PlatformFeature::Observability => &mut self.observability,
        };
        *slot = choice;
    }

    /// This selection with `feature` on, filled by the catalog's default provider.
    pub fn with(mut self, feature: PlatformFeature) -> Self {
        self.set(feature, Some(FeatureChoice::Default));
        self
    }
}

/// (De)serializes an `Option<FeatureChoice>`: `true` is the default provider, a string names
/// one, and `false` is off.
mod choice {
    use super::*;

    #[derive(Serialize, Deserialize)]
    #[serde(untagged)]
    enum Wire {
        On(bool),
        Provider(String),
    }

    pub(super) fn serialize<S: Serializer>(
        value: &Option<FeatureChoice>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            None => Wire::On(false),
            Some(FeatureChoice::Default) => Wire::On(true),
            Some(FeatureChoice::Provider(id)) => Wire::Provider(id.as_str().to_string()),
        }
        .serialize(serializer)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<FeatureChoice>, D::Error> {
        Ok(match Option::<Wire>::deserialize(deserializer)? {
            None | Some(Wire::On(false)) => None,
            Some(Wire::On(true)) => Some(FeatureChoice::Default),
            Some(Wire::Provider(id)) => Some(FeatureChoice::Provider(ModuleId::from(id))),
        })
    }
}

/// The protocols platform features are spoken over. An open set, like [`Role`]: a catalog may
/// use any string, and these are the ones the baseline catalog speaks.
pub mod protocol {
    /// An HTTP forward-auth check: the gateway asks the provider about each request and
    /// forwards the identity it asserts.
    pub const FORWARD_AUTH: &str = "forward-auth";
    /// Envoy's gRPC external-authorization API (`envoy.service.auth.v3.Authorization`).
    pub const ENVOY_EXT_AUTHZ_GRPC: &str = "envoy-ext-authz-grpc";
    /// olai-auth's policy-decision API (`POST /v1/authorize`).
    pub const OLAI_AUTHORIZE: &str = "olai-authorize";
    /// OTLP over gRPC.
    pub const OTLP_GRPC: &str = "otlp-grpc";
    /// OTLP over HTTP (protobuf), at the traces path.
    pub const OTLP_HTTP: &str = "otlp-http";
    /// The identity the gateway asserts in its trusted `Remote-*` headers, used by an
    /// integration [via the gateway](super::IntegrationSpec::via_gateway).
    pub const GATEWAY_HEADERS: &str = "gateway-headers";
}

/// A protocol a platform-feature provider serves, and where (see [`Provides::protocols`]).
///
/// [`Provides::protocols`]: crate::Provides::protocols
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolOffer {
    /// The protocol (see [`protocol`]).
    pub protocol: String,
    /// The provider's endpoint serving it (an [`Endpoint::id`](crate::Endpoint::id) of one of
    /// its services).
    pub endpoint: String,
    /// The path the protocol is served at on that endpoint (e.g. `/v1/traces`); empty for its
    /// root.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub path: String,
}

impl ProtocolOffer {
    /// `protocol` served at the root of `endpoint`.
    pub fn new(protocol: &str, endpoint: &str) -> Self {
        ProtocolOffer {
            protocol: protocol.into(),
            endpoint: endpoint.into(),
            path: String::new(),
        }
    }

    /// This offer, served at `path`.
    pub fn at(mut self, path: &str) -> Self {
        self.path = path.into();
        self
    }
}

/// A platform feature a module can integrate with (see [`Provides::integrations`]).
///
/// [`Provides::integrations`]: crate::Provides::integrations
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntegrationSpec {
    /// The feature.
    pub feature: PlatformFeature,
    /// The protocols the module can speak to the feature's provider, most preferred first. The
    /// integration uses the first one the provider serves.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub protocols: Vec<String>,
    /// The features this integration only works together with, in the same module (a policy
    /// check decides on the identity the module knows).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<PlatformFeature>,
    /// The module takes the feature from the gateway in front of it rather than from the
    /// provider: it trusts the identity the gateway asserts (its `Remote-*` headers). Active
    /// whenever the gateway's own integration with the feature is.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub via_gateway: bool,
    /// Why the module can't integrate with a provider that serves none of its protocols, when
    /// there is more to say than that (e.g. a second requirement the provider can't meet).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl IntegrationSpec {
    /// Integrate with `feature` over `protocols`, most preferred first.
    pub fn new(feature: PlatformFeature, protocols: &[&str]) -> Self {
        IntegrationSpec {
            feature,
            protocols: protocols.iter().map(|p| (*p).to_string()).collect(),
            requires: Vec::new(),
            via_gateway: false,
            note: None,
        }
    }

    /// Integrate with `feature` through the gateway in front of the module (see
    /// [`via_gateway`](Self::via_gateway)).
    pub fn via_gateway(feature: PlatformFeature) -> Self {
        IntegrationSpec {
            via_gateway: true,
            ..Self::new(feature, &[])
        }
    }

    /// This spec, needing `features` too.
    pub fn requiring(mut self, features: &[PlatformFeature]) -> Self {
        self.requires = features.to_vec();
        self
    }

    /// This spec, with a [`note`](Self::note).
    pub fn noting(mut self, note: &str) -> Self {
        self.note = Some(note.into());
        self
    }
}

/// A platform-feature integration the planner turned on for a module: what its templates read
/// as `integrations.<feature>` (see [`RenderCtx::integrations`](crate::RenderCtx::integrations)).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveIntegration {
    /// The feature's provider.
    pub provider: ModuleId,
    /// The protocol spoken (see [`protocol`]).
    pub protocol: String,
    /// Where the module reaches the provider over that protocol, from inside the stack: an
    /// absolute URL (`http://jaeger:4317`). `None` for an integration
    /// [via the gateway](IntegrationSpec::via_gateway).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_read_true_false_and_provider_ids() {
        let platform: PlatformSelection =
            toml::from_str("identity = true\npolicy = \"opa\"\nobservability = false\n").unwrap();
        assert_eq!(platform.identity, Some(FeatureChoice::Default));
        assert_eq!(
            platform.policy,
            Some(FeatureChoice::Provider(ModuleId::from("opa")))
        );
        assert_eq!(platform.observability, None);
        // Off features are not written.
        assert_eq!(
            toml::to_string(&platform).unwrap(),
            "identity = true\npolicy = \"opa\"\n"
        );
    }

    #[test]
    fn features_map_to_their_roles() {
        for feature in PlatformFeature::ALL {
            assert_eq!(PlatformFeature::of_role(feature.role()), Some(feature));
        }
        assert_eq!(PlatformFeature::of_role(Role::GATEWAY), None);
    }
}
