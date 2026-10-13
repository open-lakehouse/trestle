//! Phase 1 — **augment**: fold the selection's implied modules and provider preferences in
//! before the graph is resolved.
//!
//! Forward-auth is a gateway knob, not a separately-selected module, so when it resolves on the
//! auth provider joins the selection here (adding a module changes the graph, so this must run
//! before resolution). A provider the user picked directly also becomes the preferred provider
//! of its role, so it satisfies that role's demands ahead of the catalog default.

use std::borrow::Cow;

use super::gateway::{gateway_auth_enabled, selected_auth_provider, selected_policy_provider};
use super::{PlanCtx, PlanError, Selection, demands::with_selected_providers};
use crate::catalog::Catalog;
use crate::catalog::module::ModuleId;
use crate::model::role::Role;

/// The selection and context the rest of the planner runs against, plus the auth / policy
/// providers to wire into the gateway.
pub(super) struct Augmented<'a> {
    /// The selection, with the auth provider added when forward-auth is on.
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
pub(super) fn augment<'a>(
    selection: &'a Selection,
    catalog: &Catalog,
    ctx: &'a PlanCtx,
) -> Result<Augmented<'a>, PlanError> {
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
    let policy_provider = selected_policy_provider(selection, catalog);
    if let (Some(module), None) = (&policy_provider, &auth_provider) {
        return Err(PlanError::PolicyRequiresAuth {
            module: module.clone(),
        });
    }

    let selection = match &auth_provider {
        Some(provider) if !selection.modules.contains(provider) => {
            let mut s = selection.clone();
            s.modules.push(provider.clone());
            Cow::Owned(s)
        }
        _ => Cow::Borrowed(selection),
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
