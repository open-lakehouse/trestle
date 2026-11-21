//! Phase 1 — **augment**: fold the selection's implied modules and provider preferences in
//! before the graph is resolved.
//!
//! Unless the selection turns it off, every environment has a gateway, so the `gateway`-role
//! provider joins the selection here whether or not it was picked. The providers of the
//! selected [platform features](super::Selection::platform) join too, as does every hosted
//! provider the selection [configures](super::Selection::hosted) (adding a module changes the
//! graph, so this must run before resolution). A provider the user picked directly also becomes
//! the preferred provider of its role, so it satisfies that role's demands ahead of the catalog
//! default.

use std::borrow::Cow;
use std::collections::BTreeMap;

use super::gateway::gateway_provider;
use super::{PlanCtx, PlanError, Selection, demands::with_selected_providers};
use crate::catalog::Catalog;
use crate::catalog::module::ModuleId;
use crate::manifest::migrate_platform;
use crate::model::platform::{FeatureChoice, PlatformFeature};
use crate::model::role::Role;

/// The selection and context the rest of the planner runs against, plus the providers the
/// gateway and the integrations are wired to.
pub(super) struct Augmented<'a> {
    /// The selection, with the gateway, the platform-feature providers and the configured
    /// hosted providers added.
    pub selection: Cow<'a, Selection>,
    /// The context, with directly-selected providers preferred for their roles.
    pub ctx: Cow<'a, PlanCtx>,
    /// The gateway module, when the environment has one.
    pub gateway: Option<ModuleId>,
    /// The provider of each selected platform feature.
    pub platform: BTreeMap<PlatformFeature, ModuleId>,
}

/// The selection in its current form: its legacy gateway settings moved to platform features
/// (see [`migrate_platform`]), and any platform-feature provider it selects as a module or
/// capability made that feature's provider instead. The planner adds the providers itself, so
/// listing one is the same as turning its feature on.
pub(super) fn normalize_platform<'a>(
    selection: &'a Selection,
    catalog: &Catalog,
) -> Cow<'a, Selection> {
    let listed = |s: &Selection| {
        s.modules.iter().any(|m| {
            catalog
                .get(m)
                .and_then(|m| m.provider_of())
                .and_then(PlatformFeature::of_role)
                .is_some()
        }) || s
            .capabilities
            .iter()
            .any(|c| PlatformFeature::of_role(c).is_some())
    };
    let mut normalized = selection.clone();
    let migrated = migrate_platform(&mut normalized);
    if !migrated && !listed(&normalized) {
        return Cow::Borrowed(selection);
    }
    let modules = std::mem::take(&mut normalized.modules);
    for module in modules {
        let feature = catalog
            .get(&module)
            .and_then(|m| m.provider_of())
            .and_then(PlatformFeature::of_role);
        match feature {
            Some(feature) => {
                if normalized.platform.get(feature).is_none() {
                    normalized
                        .platform
                        .set(feature, Some(FeatureChoice::Provider(module)));
                }
            }
            None => normalized.modules.push(module),
        }
    }
    let capabilities = std::mem::take(&mut normalized.capabilities);
    for capability in capabilities {
        match PlatformFeature::of_role(&capability) {
            Some(feature) => {
                if normalized.platform.get(feature).is_none() {
                    normalized
                        .platform
                        .set(feature, Some(FeatureChoice::Default));
                }
            }
            None => normalized.capabilities.push(capability),
        }
    }
    Cow::Owned(normalized)
}

/// The provider of each platform feature `selection` turns on. Errors with
/// [`PlanError::UnknownPlatformProvider`] for a feature naming a module that doesn't provide
/// it, or one the catalog has no provider for.
pub(super) fn platform_providers(
    selection: &Selection,
    catalog: &Catalog,
) -> Result<BTreeMap<PlatformFeature, ModuleId>, PlanError> {
    let mut providers = BTreeMap::new();
    for feature in PlatformFeature::ALL {
        let Some(choice) = selection.platform.get(feature) else {
            continue;
        };
        let candidates = catalog.providers_of(feature.role());
        let provider = match choice {
            FeatureChoice::Default => {
                candidates.first().map(|p| (*p).clone()).ok_or_else(|| {
                    PlanError::UnknownPlatformProvider {
                        feature,
                        reason: format!("the catalog has no `{}` provider", feature.role()),
                    }
                })?
            }
            FeatureChoice::Provider(id) if candidates.contains(&id) => id.clone(),
            FeatureChoice::Provider(id) => {
                return Err(PlanError::UnknownPlatformProvider {
                    feature,
                    reason: format!("`{id}` is not a `{}` provider", feature.role()),
                });
            }
        };
        providers.insert(feature, provider);
    }
    Ok(providers)
}

/// Run the augment phase (see the [module docs](self)) on a
/// [normalized](normalize_platform) selection. Errors with [`PlanError::PolicyRequiresIdentity`]
/// when policy is on without identity.
///
/// With the gateway off, `emulating` still adds it: some module in the graph emulates public
/// hostnames, which only the gateway's in-network TLS listener answers (see
/// [`route`](super::route)).
pub(super) fn augment<'a>(
    selection: &'a Selection,
    catalog: &Catalog,
    ctx: &'a PlanCtx,
    emulating: bool,
) -> Result<Augmented<'a>, PlanError> {
    let platform = platform_providers(selection, catalog)?;
    if let (Some(module), None) = (
        platform.get(&PlatformFeature::Policy),
        platform.get(&PlatformFeature::Identity),
    ) {
        return Err(PlanError::PolicyRequiresIdentity {
            module: module.clone(),
        });
    }
    if !selection.gateway.enabled {
        require_no_gateway_features(selection, &platform, catalog, ctx)?;
    }

    // The gateway, the platform-feature providers, and every configured hosted provider join the
    // selection like any directly selected module (a hosted provider satisfies its role's
    // demands and appears in the topology; it is just not deployed).
    let gateway = (selection.gateway.enabled || emulating)
        .then(|| gateway_provider(selection, catalog, ctx))
        .flatten();
    let implied: Vec<&ModuleId> = gateway
        .iter()
        .chain(platform.values())
        .chain(selection.hosted.keys())
        .filter(|m| !selection.modules.contains(m))
        .collect();
    let selection = if implied.is_empty() {
        Cow::Borrowed(selection)
    } else {
        let mut s = selection.clone();
        for module in implied {
            if !s.modules.contains(module) {
                s.modules.push(module.clone());
            }
        }
        Cow::Owned(s)
    };
    let ctx = match with_selected_providers(ctx, &selection, catalog) {
        Some(preferred) => Cow::Owned(preferred),
        None => Cow::Borrowed(ctx),
    };

    Ok(Augmented {
        selection,
        ctx,
        gateway,
        platform,
    })
}

/// Reject a gateway-less selection that asks for something only the gateway provides: identity
/// or policy, an app upstream (the gateway's catch-all), or a `gateway`-role module selected
/// explicitly. Emulated public hostnames are only known once the graph is resolved, so the route
/// phase checks those.
fn require_no_gateway_features(
    selection: &Selection,
    platform: &BTreeMap<PlatformFeature, ModuleId>,
    catalog: &Catalog,
    ctx: &PlanCtx,
) -> Result<(), PlanError> {
    let required = |reason: String| Err(PlanError::GatewayRequired { reason });
    if let Some(module) = selection
        .modules
        .iter()
        .find(|m| catalog.providers_of(Role::GATEWAY).contains(m))
    {
        return required(format!("selecting the gateway module `{module}`"));
    }
    for feature in [PlatformFeature::Identity, PlatformFeature::Policy] {
        if platform.contains_key(&feature) {
            return required(feature.to_string());
        }
    }
    if ctx.app.is_some() {
        return required("an app upstream".into());
    }
    Ok(())
}
