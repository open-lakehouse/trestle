//! Phase 1 — **augment**: fold the selection's implied modules and provider preferences in
//! before the graph is resolved.
//!
//! Unless the selection turns it off, every environment has a gateway, so the `gateway`-role
//! provider joins the selection here whether or not it was picked. Sign-in and policy are [gateway settings](super::GatewaySettings),
//! not separately-selected modules, so when they resolve on their providers join too, as does
//! every hosted provider the selection [configures](super::Selection::hosted) (adding a module
//! changes the graph, so this must run before resolution). A provider the user picked directly also becomes the preferred provider
//! of its role, so it satisfies that role's demands ahead of the catalog default.

use std::borrow::Cow;

use super::gateway::{
    gateway_auth_enabled, gateway_provider, policy_provider, selected_auth_provider,
};
use super::{PlanCtx, PlanError, Selection, demands::with_selected_providers};
use crate::catalog::Catalog;
use crate::catalog::module::ModuleId;
use crate::model::role::Role;

/// The selection and context the rest of the planner runs against, plus the auth / policy
/// providers to wire into the gateway.
pub(super) struct Augmented<'a> {
    /// The selection, with the gateway, the auth and policy providers (when on) and the
    /// configured hosted providers added.
    pub selection: Cow<'a, Selection>,
    /// The context, with directly-selected providers preferred for their roles.
    pub ctx: Cow<'a, PlanCtx>,
    /// The single auth-role provider to wire, resolved once by role and reused for both the
    /// selection augmentation and the gateway's `AuthConfig` — so the two never diverge (an
    /// `ext_authz` filter pointed at a provider that isn't in the graph). `None` when the knob
    /// is off or the catalog has no `auth`-role provider; in the latter case auth stays off
    /// entirely rather than gating the gateway behind a phantom upstream.
    pub auth_provider: Option<ModuleId>,
    /// The selected policy engine, if any. It authorizes the identity forward-auth asserts,
    /// so it requires `auth_provider`.
    pub policy_provider: Option<ModuleId>,
}

/// Run the augment phase (see the [module docs](self)).
///
/// The forward-auth knob lives on whichever module fills the `gateway` role, and the provider
/// is whichever fills the `auth` role — both looked up by role, so a differently-named gateway
/// or provider still works. An explicitly selected provider wins over the catalog's first
/// `auth`-role provider. Errors with [`PlanError::PolicyRequiresAuth`] when a policy engine is
/// selected without forward-auth.
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
    if !selection.gateway.enabled {
        require_no_gateway_features(selection, catalog, ctx)?;
    }
    let auth_provider: Option<ModuleId> = if gateway_auth_enabled(selection, catalog)? {
        selected_auth_provider(selection, catalog).or_else(|| {
            catalog
                .providers_of(Role::AUTH)
                .first()
                .map(|p| (*p).clone())
        })
    } else {
        None
    };
    let policy_provider = policy_provider(selection, catalog);
    if let (Some(module), None) = (&policy_provider, &auth_provider) {
        return Err(PlanError::PolicyRequiresAuth {
            module: module.clone(),
        });
    }

    // The gateway, the auth and policy providers, and every configured hosted provider join the
    // selection like any directly selected module (a hosted provider satisfies its role's
    // demands and appears in the topology; it is just not deployed).
    let gateway = (selection.gateway.enabled || emulating)
        .then(|| gateway_provider(selection, catalog, ctx))
        .flatten();
    let implied: Vec<&ModuleId> = gateway
        .iter()
        .chain(&auth_provider)
        .chain(&policy_provider)
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
        auth_provider,
        policy_provider,
    })
}

/// Reject a gateway-less selection that asks for something only the gateway provides: sign-in
/// or policy (in any of their legacy forms too), an app upstream (the gateway's catch-all), or
/// a `gateway`-role module selected explicitly. Emulated public hostnames are only known once
/// the graph is resolved, so the route phase checks those.
fn require_no_gateway_features(
    selection: &Selection,
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
    if gateway_auth_enabled(selection, catalog)? {
        return required("sign-in".into());
    }
    if policy_provider(selection, catalog).is_some() {
        return required("policy".into());
    }
    if ctx.app.is_some() {
        return required("an app upstream".into());
    }
    Ok(())
}
