//! Phase 2 — **settle**: resolve the module graph and fix every per-module fact the later
//! phases read — each demand's provider and typed connection, each module's knobs and
//! services, and the resources each provider provisions.
//!
//! Everything here is computed exactly once; routing, binding and rendering read these caches
//! instead of re-resolving.

use std::collections::BTreeMap;

use super::demands::{
    ChosenProviders, check_role_exclusivity, provisioned_by_provider, resolve_demand_providers,
    resolve_with_demands,
};
use super::external::{Externals, ResolveExternalsError, resolve_externals};
use super::knobs::resolve_module_knobs;
use super::resolve::ResolvedGraph;
use super::{PlanCtx, PlanError, Selection, names};
use crate::catalog::Catalog;
use crate::catalog::module::{ModuleId, ResolvedKnobs, StackFacts};
use crate::model::endpoint::RouteIntent;
use crate::model::placement::Placement;
use crate::model::role::ServiceSpec;

/// The settled environment: the graph plus every fact resolved over it.
pub(super) struct Settled {
    /// The resolved, ordered module graph (auto-provisioned providers included).
    pub graph: ResolvedGraph,
    /// Each demand's chosen provider and resolved connection, keyed by `(module, demand index)`.
    pub chosen: ChosenProviders,
    /// Each module's knobs, resolved to canonical values.
    pub knobs: BTreeMap<ModuleId, ResolvedKnobs>,
    /// Stack-wide facts a module's topology may depend on.
    pub stack_facts: StackFacts,
    /// Each module's services, resolved against its knobs and the stack facts.
    pub services: BTreeMap<ModuleId, Vec<ServiceSpec>>,
    /// Every resource each provider provisions, keyed by provider.
    pub provisioned: BTreeMap<ModuleId, Vec<String>>,
    /// The external providers, resolved.
    pub externals: Externals,
}

/// Run the settle phase (see the [module docs](self)). `gateway_auth` is whether forward-auth
/// is wired (an auth provider was resolved by the augment phase).
pub(super) fn settle(
    selection: &Selection,
    catalog: &Catalog,
    ctx: &PlanCtx,
    gateway_auth: bool,
) -> Result<Settled, PlanError> {
    // Resolve the selection, auto-provisioning a provider for every resource demand
    // (a demanded relational store, object store, …) before anything else runs.
    let graph = resolve_with_demands(selection, catalog, ctx)?;

    // External providers: resolve their connections (and secret references) up front.
    let externals =
        resolve_externals(&selection.external, ctx.secrets.as_ref()).map_err(|e| match e {
            ResolveExternalsError::Unresolved(vars) => PlanError::UnresolvedSecrets { vars },
            ResolveExternalsError::Invalid { module, reason } => {
                PlanError::InvalidExternal { module, reason }
            }
        })?;
    // An external module stands in for a provider: it must provision a resource of its
    // connection's kind. These are the roles the external connection serves.
    let mut external_roles: BTreeMap<ModuleId, Vec<String>> = BTreeMap::new();
    for (id, resource) in &selection.external {
        let module = graph
            .module(id)
            .expect("external modules joined the selection");
        let roles: Vec<String> = module
            .provides()
            .resource_kinds
            .iter()
            .filter(|(_, template)| resource.connection.matches(template))
            .map(|(role, _)| role.clone())
            .collect();
        if roles.is_empty() {
            return Err(PlanError::InvalidExternal {
                module: id.clone(),
                reason: "it provisions no resource of the connection's kind".into(),
            });
        }
        external_roles.insert(id.clone(), roles);
    }

    // Every module the target runs — selected or auto-provisioned, not external — must have a
    // rendering for it.
    if crate::target::builtin(&ctx.target).is_none() {
        return Err(PlanError::UnknownTarget(ctx.target.clone()));
    }
    if let Some(module) = graph
        .nodes
        .iter()
        .find(|m| !externals.contains(m.id()) && m.render_for(&ctx.target).is_none())
    {
        return Err(PlanError::UnsupportedOnTarget {
            module: module.id().clone(),
            target: ctx.target.clone(),
        });
    }

    // Every name the plan provisions reaches a provider's init job verbatim, so check them all
    // (module demands and environment-level extras alike) before anything renders.
    for module in &graph.nodes {
        for demand in module.needs() {
            names::check_resource_name(&demand.resource, &demand.name)?;
        }
    }
    for extra in &selection.extra_resources {
        names::check_resource_name(&extra.resource, &extra.name)?;
    }

    // Resolve each demand's provider and its typed connection exactly once, up front:
    // `choose_provider` is a pure function of `(ctx, catalog, demand)`, so the downstream
    // passes (role exclusivity, provisioning, binding) all read the same chosen provider and
    // resolved connection from here instead of re-running the selection (and its per-call
    // catalog scan) at each site. A connection variant's fields are mandatory, so provider
    // completeness is a compile-time guarantee — no runtime contract check is needed.
    let mut chosen = resolve_demand_providers(&graph, catalog, ctx)?;
    // A demand served by an external provider binds the external connection instead.
    for ((module, idx), resolved) in chosen.iter_mut() {
        let Some(external) = externals.modules.get(&resolved.provider) else {
            continue;
        };
        let demand = &graph.module(module).expect("in graph").needs()[*idx];
        if external_roles[&resolved.provider].contains(&demand.resource) {
            resolved.connection = external.template.resolve(&demand.name);
        }
    }

    // At most one provider per resource role may end up in an environment — unless a
    // consumer explicitly pinned each (the sanctioned multi-store case).
    check_role_exclusivity(&graph, selection)?;

    // Resolve each module's knobs to their canonical values once, up front, and compute each
    // module's services from them. Knob resolution depends only on the knob defaults and the
    // selection overrides (never on services), so there is no circularity: the services a
    // module emits may *vary* with its knobs, but the knobs are settled first.
    let knobs = resolve_module_knobs(&graph, selection)?;
    let stack_facts = StackFacts { gateway_auth };
    let services = graph
        .nodes
        .iter()
        .map(|module| {
            let mut services = module.services_in(&knobs[module.id()], &stack_facts);
            // An external provider's services are reached directly at its origin, never routed.
            if let Some(external) = externals.modules.get(module.id()) {
                for service in &mut services {
                    service.placement = Placement::External {
                        origin: external.origin.clone(),
                    };
                    for endpoint in &mut service.endpoints {
                        endpoint.intent = RouteIntent::Internal;
                    }
                }
            }
            (module.id().clone(), services)
        })
        .collect();

    let provisioned = provisioned_by_provider(&graph, &chosen, selection, catalog, ctx)?;

    Ok(Settled {
        graph,
        chosen,
        knobs,
        stack_facts,
        services,
        provisioned,
        externals,
    })
}
