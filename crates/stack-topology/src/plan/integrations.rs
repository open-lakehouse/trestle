//! Platform-feature integrations: which module integrates with which selected
//! [platform feature](crate::PlatformFeature), over which protocol, or why it doesn't.
//!
//! Resolved during the settle phase, once each module's knobs are known and before its
//! services are, so the stack facts the services read already say whether the gateway
//! authenticates. An integration is on by default: wherever the selection turns its feature on
//! and the provider serves one of the module's protocols, unless the selection opts the module
//! out ([`Selection::integrations`](super::Selection::integrations)).

use std::collections::BTreeMap;

use serde::Serialize;

use super::PlanError;
use super::resolve::ResolvedGraph;
use crate::catalog::module::{ModuleId, ResolvedKnobs};
use crate::model::placement::{HOST_GATEWAY_DNS, Placement};
use crate::model::platform::{ActiveIntegration, PlatformFeature, protocol};
use crate::model::role::ServiceSpec;

/// One module's integration with one selected platform feature.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Integration {
    /// The integrating module.
    pub module: ModuleId,
    /// The feature.
    pub feature: PlatformFeature,
    /// The feature's provider.
    pub provider: ModuleId,
    /// Whether it is on, and if not, why.
    #[serde(flatten)]
    pub state: IntegrationState,
}

/// Whether an [`Integration`] is on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum IntegrationState {
    /// On, over `protocol`.
    Active {
        /// The protocol spoken (see [`protocol`]).
        protocol: String,
        /// Where the module reaches the provider, from inside the stack (see
        /// [`ActiveIntegration::endpoint`]).
        #[serde(skip_serializing_if = "Option::is_none")]
        endpoint: Option<String>,
    },
    /// Off: the selection opts the module out.
    Disabled,
    /// Off: the module and the provider can't integrate.
    Incompatible {
        /// Why, for a UI or an error message.
        reason: String,
    },
}

impl Integration {
    /// Whether the integration is on.
    pub fn is_active(&self) -> bool {
        matches!(self.state, IntegrationState::Active { .. })
    }

    /// The integration as its module's templates read it, if it is on.
    pub fn active(&self) -> Option<ActiveIntegration> {
        match &self.state {
            IntegrationState::Active { protocol, endpoint } => Some(ActiveIntegration {
                provider: self.provider.clone(),
                protocol: protocol.clone(),
                endpoint: endpoint.clone(),
            }),
            _ => None,
        }
    }
}

/// Whether `module`'s integration with `feature` is on in `integrations`.
pub(super) fn is_active(
    integrations: &[Integration],
    module: &ModuleId,
    feature: PlatformFeature,
) -> bool {
    integrations
        .iter()
        .any(|i| &i.module == module && i.feature == feature && i.is_active())
}

/// Each module's active integrations by feature, as its templates read them.
pub(super) fn active_by_module(
    integrations: &[Integration],
) -> BTreeMap<ModuleId, BTreeMap<PlatformFeature, ActiveIntegration>> {
    let mut by_module: BTreeMap<_, BTreeMap<_, _>> = BTreeMap::new();
    for integration in integrations {
        if let Some(active) = integration.active() {
            by_module
                .entry(integration.module.clone())
                .or_default()
                .insert(integration.feature, active);
        }
    }
    by_module
}

/// Resolve every module's integrations with the `platform` features' providers.
///
/// `gateway` is the stack's gateway module, through which a
/// [`via_gateway`](crate::IntegrationSpec::via_gateway) integration takes its feature. Errors
/// with [`PlanError::UnknownIntegration`] for an opt-out naming a module that isn't in the
/// graph or a feature it can't integrate with.
pub(super) fn resolve_integrations(
    graph: &ResolvedGraph,
    knobs: &BTreeMap<ModuleId, ResolvedKnobs>,
    platform: &BTreeMap<PlatformFeature, ModuleId>,
    opt_outs: &BTreeMap<ModuleId, BTreeMap<PlatformFeature, bool>>,
    gateway: Option<&ModuleId>,
) -> Result<Vec<Integration>, PlanError> {
    for (module, features) in opt_outs {
        let Some(node) = graph.module(module) else {
            return Err(PlanError::UnknownIntegration {
                module: module.clone(),
                reason: "it is not in the environment".into(),
            });
        };
        let specs = node.integrations_in(&knobs[module]);
        if let Some(feature) = features
            .keys()
            .find(|f| !specs.iter().any(|s| s.feature == **f))
        {
            return Err(PlanError::UnknownIntegration {
                module: module.clone(),
                reason: format!("it can't integrate with {feature}"),
            });
        }
    }

    // Each integration with the features it requires of the same module.
    let mut entries: Vec<(Integration, Vec<PlatformFeature>)> = Vec::new();
    // Integrations via the gateway, resolved once the gateway's own are.
    let mut via_gateway = Vec::new();
    for module in &graph.nodes {
        let id = module.id();
        for spec in module.integrations_in(&knobs[id]).iter() {
            let Some(provider) = platform.get(&spec.feature) else {
                continue;
            };
            if provider == id {
                continue;
            }
            let opted_out = opt_outs
                .get(id)
                .and_then(|f| f.get(&spec.feature))
                .is_some_and(|on| !on);
            let offers = graph
                .module(provider)
                .map(|p| p.provides().protocols.as_slice())
                .unwrap_or_default();
            let state = if opted_out {
                IntegrationState::Disabled
            } else if spec.via_gateway {
                // Placeholder until the gateway's own integrations are settled.
                IntegrationState::Disabled
            } else {
                match spec
                    .protocols
                    .iter()
                    .find(|p| offers.iter().any(|o| &o.protocol == *p))
                {
                    Some(protocol) => IntegrationState::Active {
                        protocol: protocol.clone(),
                        endpoint: None,
                    },
                    None => IntegrationState::Incompatible {
                        reason: no_common_protocol(provider, &spec.protocols, spec.note.as_deref()),
                    },
                }
            };
            let entry = (
                Integration {
                    module: id.clone(),
                    feature: spec.feature,
                    provider: provider.clone(),
                    state,
                },
                spec.requires.clone(),
            );
            if spec.via_gateway && !opted_out {
                via_gateway.push(entry);
            } else {
                entries.push(entry);
            }
        }
    }
    settle_requires(&mut entries);

    let gateway_has = |entries: &[(Integration, Vec<PlatformFeature>)], feature| {
        gateway.is_some_and(|g| {
            entries
                .iter()
                .any(|(i, _)| &i.module == g && i.feature == feature && i.is_active())
        })
    };
    for (mut integration, requires) in via_gateway {
        integration.state = if gateway_has(&entries, integration.feature) {
            IntegrationState::Active {
                protocol: protocol::GATEWAY_HEADERS.into(),
                endpoint: None,
            }
        } else {
            IntegrationState::Incompatible {
                reason: format!(
                    "it takes {} from the gateway, and the gateway doesn't provide it",
                    integration.feature
                ),
            }
        };
        entries.push((integration, requires));
    }
    settle_requires(&mut entries);
    Ok(entries.into_iter().map(|(i, _)| i).collect())
}

/// The reason a module can't speak to `provider`.
fn no_common_protocol(provider: &ModuleId, protocols: &[String], note: Option<&str>) -> String {
    let mut reason = if protocols.is_empty() {
        format!("`{provider}` serves no protocol it speaks")
    } else {
        format!("`{provider}` doesn't serve {}", protocols.join(" or "))
    };
    if let Some(note) = note {
        reason.push_str("; ");
        reason.push_str(note);
    }
    reason
}

/// Turn off every active integration whose module lacks an active integration with a feature
/// it requires. Repeats until nothing changes, since turning one off can strand another.
fn settle_requires(entries: &mut [(Integration, Vec<PlatformFeature>)]) {
    loop {
        let mut stranded = None;
        for (index, (integration, requires)) in entries.iter().enumerate() {
            if !integration.is_active() {
                continue;
            }
            let missing = requires.iter().find(|feature| {
                !entries.iter().any(|(other, _)| {
                    other.module == integration.module
                        && other.feature == **feature
                        && other.is_active()
                })
            });
            if let Some(feature) = missing {
                stranded = Some((index, *feature));
                break;
            }
        }
        let Some((index, feature)) = stranded else {
            return;
        };
        entries[index].0.state = IntegrationState::Incompatible {
            reason: format!("it needs its {feature} integration on too"),
        };
    }
}

/// Fill in where each active integration reaches its provider, now that every module's
/// services are known.
pub(super) fn resolve_endpoints(
    integrations: &mut [Integration],
    graph: &ResolvedGraph,
    services: &BTreeMap<ModuleId, Vec<ServiceSpec>>,
) -> Result<(), PlanError> {
    for integration in integrations.iter_mut() {
        let IntegrationState::Active { protocol, endpoint } = &mut integration.state else {
            continue;
        };
        if protocol == protocol::GATEWAY_HEADERS {
            continue;
        }
        let misconfigured = |reason: String| PlanError::PlatformProviderMisconfigured {
            module: integration.provider.clone(),
            reason,
        };
        let offer = graph
            .module(&integration.provider)
            .and_then(|p| {
                p.provides()
                    .protocols
                    .iter()
                    .find(|o| &o.protocol == protocol)
                    .cloned()
            })
            .ok_or_else(|| misconfigured(format!("it no longer serves {protocol}")))?;
        let (service, port) = services
            .get(&integration.provider)
            .into_iter()
            .flatten()
            .find_map(|s| {
                s.endpoints
                    .iter()
                    .find(|e| e.id == offer.endpoint)
                    .map(|e| (s, e.internal_port))
            })
            .ok_or_else(|| {
                misconfigured(format!(
                    "it serves {protocol} on `{}`, which none of its services declares",
                    offer.endpoint
                ))
            })?;
        *endpoint = Some(match &service.placement {
            Placement::Container { service } => format!("http://{service}:{port}{}", offer.path),
            Placement::External { origin } => {
                format!("{}{}", origin.trim_end_matches('/'), offer.path)
            }
            Placement::Host | Placement::InProcess => {
                format!("http://{HOST_GATEWAY_DNS}:{port}{}", offer.path)
            }
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(
        module: &str,
        feature: PlatformFeature,
        active: bool,
        requires: &[PlatformFeature],
    ) -> (Integration, Vec<PlatformFeature>) {
        let state = if active {
            IntegrationState::Active {
                protocol: "x".into(),
                endpoint: None,
            }
        } else {
            IntegrationState::Disabled
        };
        (
            Integration {
                module: module.into(),
                feature,
                provider: "p".into(),
                state,
            },
            requires.to_vec(),
        )
    }

    #[test]
    fn a_requirement_strands_the_integrations_that_need_it() {
        use PlatformFeature::*;
        let mut entries = vec![
            entry("gw", Policy, true, &[Identity]),
            entry("gw", Identity, true, &[]),
            entry("svc", Policy, true, &[Identity]),
            entry("svc", Identity, false, &[]),
            // Stranded only once the policy integration it needs is.
            entry("svc", Observability, true, &[Policy]),
        ];
        settle_requires(&mut entries);
        let active: Vec<bool> = entries.iter().map(|(i, _)| i.is_active()).collect();
        assert_eq!(active, [true, true, false, false, false]);
        assert_eq!(
            entries[2].0.state,
            IntegrationState::Incompatible {
                reason: "it needs its identity integration on too".into()
            }
        );
    }
}
