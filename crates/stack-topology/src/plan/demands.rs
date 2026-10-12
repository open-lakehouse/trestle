//! Selection and demand resolution: which modules join the graph, which provider satisfies
//! each resource demand, and how a resolved connection binds into a consumer's env.

use std::collections::{BTreeMap, BTreeSet};

use super::resolve::{ExtraEdges, ResolvedGraph, resolve, resolve_with};
use super::{PlanCtx, PlanError, Selection};
use crate::catalog::Catalog;
use crate::catalog::baseline::DEP_GATE_EXTRA;
use crate::catalog::module::ModuleId;
use crate::model::connection::Connection;
use crate::render::InjectedEnv;

/// Resolve a selection into a flat list of directly-selected module ids (capabilities
/// mapped through the catalog's provider index), preserving order and deduplicating.
pub(super) fn resolve_selection(
    selection: &Selection,
    catalog: &Catalog,
) -> Result<Vec<ModuleId>, PlanError> {
    let mut out: Vec<ModuleId> = Vec::new();
    let push = |id: ModuleId, out: &mut Vec<ModuleId>| {
        if !out.contains(&id) {
            out.push(id);
        }
    };
    for id in &selection.modules {
        push(id.clone(), &mut out);
    }
    for cap in &selection.capabilities {
        let providers = catalog.providers_of(cap);
        if providers.is_empty() {
            return Err(PlanError::UnknownCapability(cap.clone()));
        }
        for id in providers {
            push(id.clone(), &mut out);
        }
    }
    Ok(out)
}

/// Resolve a selection, growing it until every resource demand is satisfied.
///
/// A module's [`ResourceDemand`] names a resource *kind*; the planner finds the catalog
/// module that provisions it and adds it to the selection if absent. Because a
/// just-added provider may itself `require` or `need` more, this is a fixed point:
/// resolve → scan demands → add missing providers → re-resolve, until a pass adds
/// nothing. The `requires` closure / topo-sort / cycle detection all live in
/// [`resolve`]; this only decides *which* modules to feed it.
pub(super) fn resolve_with_demands(
    selection: &Selection,
    catalog: &Catalog,
    ctx: &PlanCtx,
) -> Result<ResolvedGraph, PlanError> {
    // Each consuming module's demanded providers, recorded as it is discovered, so the
    // final resolve can treat "needs a resource from X" as a dependency edge on X
    // (provider ordered before consumer, like a `requires`). These are passed to `resolve`
    // as `extra_edges` rather than folded into any module — a module is never mutated.
    let mut demand_edges: ExtraEdges = ExtraEdges::new();
    let mut selected = resolve_selection(selection, catalog)?;

    // Fixed point: resolve → scan demands → add missing providers → repeat. Bounded by
    // the catalog size (each iteration adds ≥1 module).
    for _ in 0..=catalog.modules().len() {
        let graph = resolve(&selected, catalog.modules())?;
        let mut added = false;
        for module in &graph.nodes {
            for demand in module.needs() {
                let provider = choose_provider(
                    ctx,
                    catalog,
                    &demand.resource,
                    demand.provider.as_ref(),
                    Some(module.id()),
                )?;
                let edges = demand_edges.entry(module.id().clone()).or_default();
                if !edges.contains(&provider) {
                    edges.push(provider.clone());
                }
                if !selected.contains(&provider) {
                    selected.push(provider);
                    added = true;
                }
            }
        }
        // Environment-level extra resources have no consuming module, so they add their
        // provider to the selection (becoming a graph node) but contribute no dependency
        // edge — nothing gates on these names at startup, and the providers
        // (postgres/rustfs/azurite) carry no `requires` of their own. This is what lets
        // an extra bucket pull in an object-store provider even when no module demands one.
        for extra in &selection.extra_resources {
            let provider =
                choose_provider(ctx, catalog, &extra.resource, extra.provider.as_ref(), None)?;
            if !selected.contains(&provider) {
                selected.push(provider);
                added = true;
            }
        }
        if !added {
            // Final resolve with the demand edges threaded in, so providers start before
            // the consumers that demand their resources.
            return resolve_with(&selected, catalog.modules(), &demand_edges).map_err(Into::into);
        }
    }
    // Unreachable in practice (the loop bound exceeds the max modules addable).
    resolve_with(&selected, catalog.modules(), &demand_edges).map_err(Into::into)
}

/// `ctx` with the providers the user selected appended to its
/// [`provider_preference`](PlanCtx::provider_preference), or `None` if that adds nothing.
///
/// Every resource kind a directly-selected module provides gets that module as a preference
/// (in selection order), after any the environment already prefers. Without this, selecting a
/// non-default provider (e.g. Azurite for object storage) would still pull in the catalog
/// default to satisfy other modules' demands, and the two would clash.
pub(super) fn with_selected_providers(
    ctx: &PlanCtx,
    selection: &Selection,
    catalog: &Catalog,
) -> Option<PlanCtx> {
    let mut preference = ctx.provider_preference.clone();
    let mut added = false;
    for id in &selection.modules {
        let Some(module) = catalog.get(id) else {
            continue;
        };
        for kind in module.provides().resource_kinds.keys() {
            let order = preference.entry(kind.clone()).or_default();
            if !order.contains(id) {
                order.push(id.clone());
                added = true;
            }
        }
    }
    added.then(|| PlanCtx {
        provider_preference: preference,
        ..ctx.clone()
    })
}

/// Choose the provider module that satisfies a demand, in priority order:
/// 1. the demand's explicit [`provider`](crate::ResourceDemand::provider) pin;
/// 2. the first [`PlanCtx::provider_preference`] entry for the role present in the catalog
///    (the planner appends the directly-selected providers here, see
///    [`with_selected_providers`]);
/// 3. the sole provider, if the role has exactly one;
/// 4. the catalog's declared default provider for the role.
///
/// Errors with [`PlanError::UnsatisfiedDemand`] if no module provides the role, or
/// [`PlanError::AmbiguousProvider`] if several do and none of the above selects one.
/// `consumer` (when known) labels an `UnsatisfiedDemand`.
///
/// Takes the role and optional pin directly (not a [`ResourceDemand`]) so it can satisfy both a
/// module's demand and an environment-level [`ExtraResource`], which has no consumer to bind.
pub(super) fn choose_provider(
    ctx: &PlanCtx,
    catalog: &Catalog,
    role: &str,
    pin: Option<&ModuleId>,
    consumer: Option<&ModuleId>,
) -> Result<ModuleId, PlanError> {
    let candidates = catalog.providers_for(role);
    if candidates.is_empty() {
        return Err(PlanError::UnsatisfiedDemand {
            module: consumer.map(|m| m.0.clone()).unwrap_or_default(),
            resource: role.to_string(),
        });
    }
    let provides_role = |id: &ModuleId| candidates.contains(&id);

    // 1. Explicit pin.
    if let Some(pin) = pin {
        if provides_role(pin) {
            return Ok(pin.clone());
        }
        // A pin that doesn't provide the role is unsatisfiable for this demand.
        return Err(PlanError::UnsatisfiedDemand {
            module: consumer.map(|m| m.0.clone()).unwrap_or_default(),
            resource: role.to_string(),
        });
    }
    // 2. Environment preference.
    if let Some(order) = ctx.provider_preference.get(role)
        && let Some(pref) = order.iter().find(|id| provides_role(id))
    {
        return Ok(pref.clone());
    }
    // 3. Unique provider.
    if candidates.len() == 1 {
        return Ok(candidates[0].clone());
    }
    // 4. Catalog default.
    if let Some(def) = catalog.default_provider_for(role)
        && provides_role(def)
    {
        return Ok(def.clone());
    }
    // Otherwise genuinely ambiguous.
    let mut providers: Vec<ModuleId> = candidates.into_iter().cloned().collect();
    providers.sort();
    Err(PlanError::AmbiguousProvider {
        resource: role.to_string(),
        providers,
    })
}

/// Bind a demand's resolved [`Connection`] into `env` by typed field, per the demand's
/// [`ConnectionBinding`](crate::ConnectionBinding).
///
/// Each `(field, key)` pair sets `key` to the connection's value for `field`. A field the
/// connection variant does not carry is a [`PlanError::UnboundConnectionField`].
pub(super) fn bind_connection(
    env: &mut InjectedEnv,
    consumer: &ModuleId,
    demand: &crate::catalog::module::ResourceDemand,
    connection: &Connection,
) -> Result<(), PlanError> {
    for (field, key) in &demand.bind.bind {
        let value = connection
            .field(*field)
            .ok_or_else(|| PlanError::UnboundConnectionField {
                module: consumer.0.clone(),
                resource: demand.resource.clone(),
                field: *field,
            })?;
        env.set(key, value);
    }
    Ok(())
}

/// The provider chosen for a demand and the typed connection it resolves to.
#[derive(Clone)]
pub(super) struct ResolvedDemand {
    /// The provider module chosen to satisfy the demand.
    pub(super) provider: ModuleId,
    /// The provider's connection template, resolved for the demanded resource name.
    pub(super) connection: Connection,
}

/// The resolved provider + connection for each demand, keyed by `(demanding module id,
/// demand index)`.
///
/// Computed once by [`resolve_demand_providers`] and shared by the planner's passes so each
/// demand's provider is selected — and its connection resolved — a single time (rather than
/// re-running [`choose_provider`] and re-substituting `{name}` at every consumer).
pub(super) type ChosenProviders = BTreeMap<(ModuleId, usize), ResolvedDemand>;

/// Resolve the provider and typed connection for every demand in the graph, once.
///
/// A connection variant's fields are all mandatory, so a chosen provider cannot render an
/// incomplete connection — completeness is a compile-time guarantee, replacing the old
/// runtime role-contract validation.
pub(super) fn resolve_demand_providers(
    graph: &ResolvedGraph,
    catalog: &Catalog,
    ctx: &PlanCtx,
) -> Result<ChosenProviders, PlanError> {
    let mut chosen = ChosenProviders::new();
    for module in &graph.nodes {
        for (idx, demand) in module.needs().iter().enumerate() {
            let provider_id = choose_provider(
                ctx,
                catalog,
                &demand.resource,
                demand.provider.as_ref(),
                Some(module.id()),
            )?;
            let provider = catalog
                .get(&provider_id)
                .expect("provider id is in catalog");
            let template = provider
                .provides()
                .resource_kinds
                .get(&demand.resource)
                .expect("chosen provider provisions the demanded role");
            let connection = template.resolve(&demand.name);
            chosen.insert(
                (module.id().clone(), idx),
                ResolvedDemand {
                    provider: provider_id,
                    connection,
                },
            );
        }
    }
    Ok(chosen)
}

/// Reject an environment that contains two or more providers of the same resource role,
/// unless every extra provider was reached by an explicit per-demand
/// [`provider`](crate::ResourceDemand::provider) pin.
///
/// `choose_provider` already selects exactly one provider per demand, so a same-role clash
/// only arises when a *second* provider is in the graph for an unrelated reason — directly
/// selected, or pulled in by `requires`. That is the case this rejects (with
/// [`PlanError::ConflictingRoleProviders`]): selecting both RustFS and Azurite with no
/// pins is ambiguous, and the planner will not silently drop one. The exception is the
/// deliberate multi-store shape — a consumer that pins each demand's provider, or an
/// environment-level [`ExtraResource`] that pins one — where every provider beyond the first is
/// accounted for by a pin.
pub(super) fn check_role_exclusivity(
    graph: &ResolvedGraph,
    selection: &Selection,
) -> Result<(), PlanError> {
    // The (role, provider) pairs a demand or an extra resource explicitly pinned — sanctioned.
    let mut pinned: BTreeSet<(String, ModuleId)> = BTreeSet::new();
    for module in &graph.nodes {
        for demand in module.needs() {
            if let Some(p) = &demand.provider {
                pinned.insert((demand.resource.clone(), p.clone()));
            }
        }
    }
    for extra in &selection.extra_resources {
        if let Some(p) = &extra.provider {
            pinned.insert((extra.resource.clone(), p.clone()));
        }
    }

    // Providers of each role present in the resolved graph (a node provides a role if it
    // declares it under `resource_kinds`, or its `provider_of` capability names it).
    let mut by_role: BTreeMap<String, Vec<ModuleId>> = BTreeMap::new();
    for module in &graph.nodes {
        let mut roles: BTreeSet<&str> = module
            .provides()
            .resource_kinds
            .keys()
            .map(String::as_str)
            .collect();
        if let Some(cap) = module.provider_of() {
            roles.insert(cap);
        }
        for role in roles {
            by_role
                .entry(role.to_string())
                .or_default()
                .push(module.id().clone());
        }
    }

    for (role, mut providers) in by_role {
        if providers.len() < 2 {
            continue;
        }
        // Allow it only if at most one provider is unpinned: every other was a deliberate pin.
        let unpinned = providers
            .iter()
            .filter(|id| !pinned.contains(&(role.clone(), (*id).clone())))
            .count();
        if unpinned > 1 {
            // Ignore a role that no demand actually targets (e.g. a `provider_of` capability
            // with no matching `resource_kinds` demand): it is not a provisioning clash.
            let demanded = graph
                .nodes
                .iter()
                .flat_map(|m| m.needs())
                .any(|d| d.resource == role);
            if !demanded {
                continue;
            }
            providers.sort();
            return Err(PlanError::ConflictingRoleProviders { role, providers });
        }
    }
    Ok(())
}

/// The typed `depends_on` gate a chosen `provider` advertises for its consumers, or `None`
/// if it declares no startup gate (nothing to wait for).
///
/// The provider declares the gate as a single `"<service>:<condition>"`
/// [`DEP_GATE_EXTRA`] value (e.g. `"db:service_healthy"`,
/// `"rustfs-init:service_completed_successfully"`); this parses it into a typed
/// [`DepGate`](crate::catalog::module::DepGate) the planner hands the consumer's render. An
/// unrecognized or missing condition defaults to
/// [`ServiceStarted`](crate::DependsCondition::ServiceStarted), the weakest compose gate.
pub(super) fn provider_dep_gate(
    catalog: &Catalog,
    provider: &ModuleId,
) -> Option<crate::catalog::module::DepGate> {
    let module = catalog.get(provider)?;
    let gate = module.provides().extras.get(DEP_GATE_EXTRA)?;
    let (service, condition) = match gate.split_once(':') {
        Some((s, c)) => (s, crate::catalog::module::DependsCondition::parse(c)),
        None => (gate.as_str(), None),
    };
    Some(crate::catalog::module::DepGate {
        service: service.to_string(),
        condition: condition.unwrap_or(crate::catalog::module::DependsCondition::ServiceStarted),
    })
}

/// The resources to provision, grouped by the *chosen provider* (deduped, in dependency
/// order). Grouping by provider — not by abstract role — is what lets one object-store
/// demand land on RustFS and another on Azurite, each provisioning on its own init.
///
/// Modules are walked in graph (dependency) order so the grouped names stay deterministic.
/// The environment-level [`extra_resources`](Selection::extra_resources) are folded in on the
/// same grouping, so the chosen provider's existing init job creates them (the names reach its
/// render as `RenderCtx.objects`). They are appended after the module-demanded names and
/// deduped, so an extra naming an already-provisioned resource is a no-op.
pub(super) fn provisioned_by_provider(
    graph: &ResolvedGraph,
    chosen: &ChosenProviders,
    selection: &Selection,
    catalog: &Catalog,
    ctx: &PlanCtx,
) -> Result<BTreeMap<ModuleId, Vec<String>>, PlanError> {
    let mut provisioned_by: BTreeMap<ModuleId, Vec<String>> = BTreeMap::new();
    let mut add = |provider: ModuleId, name: &str| {
        let names = provisioned_by.entry(provider).or_default();
        if !names.iter().any(|n| n == name) {
            names.push(name.to_string());
        }
    };
    for module in &graph.nodes {
        for (idx, demand) in module.needs().iter().enumerate() {
            add(
                chosen[&(module.id().clone(), idx)].provider.clone(),
                &demand.name,
            );
        }
    }
    for extra in &selection.extra_resources {
        let provider =
            choose_provider(ctx, catalog, &extra.resource, extra.provider.as_ref(), None)?;
        add(provider, &extra.name);
    }
    Ok(provisioned_by)
}
