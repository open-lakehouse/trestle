//! Phase 3 — **route**: assign every surface endpoint a non-colliding gateway route and build
//! the structured [`GatewayConfig`] (listeners, clusters, forward-auth, authorization, TLS).
//!
//! This is the one phase that sees every module's endpoints at once, so it is where prefixes
//! are decided. Without a gateway ([`GatewaySettings::enabled`](super::GatewaySettings::enabled)
//! off) it instead publishes each surface endpoint on its own host port (see [`route_direct`]). It also finalizes the object-store connections the settle phase resolved: a
//! gatewayed store's endpoint is rewritten to the gateway, and a store whose cloud hostnames the
//! gateway emulates carries the trust for the gateway's local CA.

use std::collections::{BTreeMap, BTreeSet};

use super::demands::ChosenProviders;
use super::gateway::{
    allocate_dedicated_ports, api_rewrite, claim, ensure_cluster, join_path, require_prefix,
    wire_app_upstream, wire_auth, wire_policy,
};
use super::impersonation::{plan_impersonation, rewrite_gatewayed_object_store_endpoints};
use super::resolve::ResolvedGraph;
use super::routing::{AssignedRoute, Listener, RoutePlan};
use super::settle::Settled;
use super::{
    GatewayConfig, GatewayFacts, GatewayRoute, ListenerConfig, PlanCtx, PlanError, SecureOrigin,
};
use crate::catalog::Catalog;
use crate::catalog::module::{ModuleId, PortMapping};
use crate::model::connection::Connection;
use crate::model::endpoint::RouteIntent;
use crate::model::placement::Placement;
use crate::model::role::ServiceSpec;

/// The routing decisions: the per-endpoint routes the address resolver reads, the gateway
/// config the Envoy renderer reads, and the gateway facts both resolve addresses against.
pub(super) struct Routed {
    /// The per-endpoint route assignments.
    pub routes: RoutePlan,
    /// The structured gateway config.
    pub gateway: GatewayConfig,
    /// The gateway's name, ports and (under forward-auth) secure origin.
    pub facts: GatewayFacts,
    /// Without a gateway, the `host:container` ports each module publishes for its surface
    /// endpoints. `None` when the gateway fronts them.
    pub direct: Option<BTreeMap<ModuleId, Vec<PortMapping>>>,
}

/// Run the route phase (see the [module docs](self)), rewriting `settled.chosen`'s gatewayed
/// and emulated object-store connections in place.
pub(super) fn route(
    settled: &mut Settled,
    catalog: &Catalog,
    ctx: &PlanCtx,
    gateway_enabled: bool,
    auth_provider: Option<&ModuleId>,
    policy_provider: Option<&ModuleId>,
) -> Result<Routed, PlanError> {
    if !gateway_enabled {
        return route_direct(settled, ctx);
    }
    let Settled {
        graph,
        chosen,
        services: services_of,
        provisioned,
        stack_facts,
        ..
    } = settled;

    // Allocate dedicated listener ports up front, before the routing loop, so a gatewayed
    // object store's connection `endpoint` can be rewritten to the gateway *before* consumers
    // bind it.
    let dedicated_ports = allocate_dedicated_ports(graph, services_of, ctx);

    // Rewrite each gatewayed object store's resolved `endpoint` (and the `BlobEndpoint=` inside
    // an Azure connection string) to the gateway origin, so consumers reach the store through
    // Envoy on its dedicated listener rather than the provider's in-network address. The
    // provider's own self-connection (used to render its fragment) is rewritten in the same
    // pass via `chosen`.
    rewrite_gatewayed_object_store_endpoints(chosen, services_of, graph, ctx, &dedicated_ports);

    let mut routes = RoutePlan::new();
    let mut gateway = GatewayConfig {
        admin_port: ctx.gateway_admin_port,
        ..GatewayConfig::default()
    };
    emulate_public_hosts(graph, provisioned, chosen, &mut gateway)?;
    // Forward-auth needs a secure, dotted origin (see `SecureOrigin`).
    if stack_facts.gateway_auth {
        gateway.secure = Some(SecureOrigin::for_env(&ctx.env_name));
    }

    let facts = GatewayFacts {
        service: ctx.gateway_service.clone(),
        internal_port: ctx.gateway_internal_port,
        host_port: ctx.gateway_host_port,
        secure: gateway.secure.clone(),
    };

    let mut shared_routes: Vec<GatewayRoute> = Vec::new();
    let mut dedicated: Vec<ListenerConfig> = Vec::new();
    // Track claimed prefixes (per listener) to detect collisions. The shared listener
    // is keyed by its port; dedicated listeners never collide (own port).
    let mut claimed: BTreeMap<String, String> = BTreeMap::new();

    // Walk modules in dependency order so emitted routes/clusters are deterministic.
    for module in &graph.nodes {
        for service in &services_of[module.id()] {
            for endpoint in &service.endpoints {
                match &endpoint.intent {
                    RouteIntent::Internal => {} // no route
                    RouteIntent::Api => {
                        // The mount prefix is the endpoint's typed `mount_prefix` (its `path`
                        // stays empty so the resolver's `join(prefix, path)` round-trips to
                        // exactly the prefix).
                        let prefix = endpoint.mount_prefix.clone().unwrap_or_default();
                        require_prefix(&prefix, service, &endpoint.id)?;
                        claim(&mut claimed, &prefix, service, &endpoint.id)?;
                        let rewrite = api_rewrite(&endpoint.rewrite, &service.base_path, &prefix);
                        let cluster = ensure_cluster(&mut gateway, service, endpoint)?;
                        shared_routes.push(GatewayRoute {
                            prefix: prefix.clone(),
                            cluster: cluster.clone(),
                            rewrite: rewrite.clone(),
                            options: endpoint.route.clone(),
                        });
                        // One rewrite value feeds both the gateway config and the
                        // route plan, keeping them in lock-step.
                        routes.assign(
                            &service.name,
                            &endpoint.id,
                            AssignedRoute {
                                prefix,
                                rewrite,
                                listener: Listener::Shared,
                                base_path: None,
                            },
                        );
                    }
                    RouteIntent::UiPrefixable => {
                        let base = service.base_path.clone();
                        // An empty base path would mount the UI at `/` — a catch-all
                        // that shadows every other route. A prefixable UI must declare
                        // a non-empty base path; if it truly serves at root it is a
                        // `UiFixed` with its own listener instead.
                        require_prefix(&base, service, &endpoint.id)?;
                        claim(&mut claimed, &base, service, &endpoint.id)?;
                        let cluster = ensure_cluster(&mut gateway, service, endpoint)?;
                        shared_routes.push(GatewayRoute {
                            prefix: base.clone(),
                            cluster: cluster.clone(),
                            rewrite: None,
                            options: endpoint.route.clone(),
                        });
                        routes.assign(
                            &service.name,
                            &endpoint.id,
                            AssignedRoute {
                                prefix: base.clone(),
                                rewrite: None,
                                listener: Listener::Shared,
                                base_path: Some(base),
                            },
                        );
                    }
                    // A fixed-path UI and a gatewayed backend (e.g. an object store) share the
                    // same routing: their own dedicated listener serving `/`, no rewrite, so
                    // the client reaches the service at the listener's origin and constructs
                    // its own URLs unprefixed.
                    RouteIntent::UiFixed | RouteIntent::Gatewayed => {
                        let port = dedicated_ports[&(service.name.clone(), endpoint.id.clone())];
                        let cluster = ensure_cluster(&mut gateway, service, endpoint)?;
                        // The dedicated listener binds the allocated port on *both* sides:
                        // Envoy listens on it inside the network (so an in-network consumer
                        // reaches `envoy:<port>`) and it is published 1:1 to the host. It
                        // forwards to the service-named cluster, which already targets the
                        // upstream's own port (`endpoint.internal_port`) via `ensure_cluster`.
                        dedicated.push(ListenerConfig {
                            host_port: port,
                            internal_port: port,
                            routes: vec![GatewayRoute {
                                prefix: "/".into(),
                                cluster: cluster.clone(),
                                rewrite: None,
                                options: endpoint.route.clone(),
                            }],
                            gated: endpoint.route.gated,
                        });
                        routes.assign(
                            &service.name,
                            &endpoint.id,
                            AssignedRoute {
                                prefix: "/".into(),
                                rewrite: None,
                                listener: Listener::Dedicated { port },
                                base_path: None,
                            },
                        );
                    }
                }
            }
        }
    }

    // The user's app, when present, becomes the gateway's catch-all: an `app` cluster and a `/`
    // route on the shared listener. The `/` route is the shortest prefix, so the
    // most-specific-first sort below settles it last — exactly where a catch-all belongs. Run
    // the same collision checks every module route gets (rather than blindly injecting), so an
    // app-vs-module conflict fails loudly here instead of as an Envoy config error at launch.
    if let Some(app) = &ctx.app {
        wire_app_upstream(app, &claimed, &mut gateway, &mut shared_routes)?;
    }

    // Order shared-listener routes most-specific-first (longer prefix wins), stable
    // within equal length by prefix so the output is deterministic.
    shared_routes.sort_by(|a, b| {
        b.prefix
            .len()
            .cmp(&a.prefix.len())
            .then_with(|| a.prefix.cmp(&b.prefix))
    });

    gateway.listeners.push(ListenerConfig {
        host_port: ctx.gateway_host_port,
        internal_port: ctx.gateway_internal_port,
        routes: shared_routes,
        gated: true,
    });
    gateway.listeners.extend(dedicated);

    // Forward-auth: when an auth provider was resolved (knob on + a provider exists), add its
    // upstream cluster and record the `AuthConfig` so the Envoy renderer gates the shared
    // listener behind `ext_authz`. The provider joined the graph in the augment phase, so it
    // is a real service; its cluster name/host/port derive from its resolved service and
    // `Internal` endpoint — not from hardcoded constants — so a differently-named or -ported
    // provider is wired correctly. The routing loop above skips it because the auth endpoint
    // is `Internal` (no route).
    if let Some(provider) = auth_provider {
        wire_auth(provider, services_of, catalog, &mut gateway)?;
    }
    // Authorization: a selected policy engine checks every request on the shared listener after
    // forward-auth (validated by the augment phase to be on).
    if let Some(provider) = policy_provider {
        wire_policy(provider, services_of, &mut gateway)?;
    }

    gateway.clusters.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(Routed {
        routes,
        gateway,
        facts,
        direct: None,
    })
}

/// Emulated public hostnames: give the gateway a TLS listener for them, and mark every
/// connection those providers vend with the trust a consumer needs for the local CA.
fn emulate_public_hosts(
    graph: &ResolvedGraph,
    provisioned: &BTreeMap<ModuleId, Vec<String>>,
    chosen: &mut ChosenProviders,
    gateway: &mut GatewayConfig,
) -> Result<(), PlanError> {
    gateway.tls = plan_impersonation(graph, provisioned, gateway)?;
    if let Some(tls) = &gateway.tls {
        let emulating: BTreeSet<&ModuleId> = graph
            .nodes
            .iter()
            .filter(|m| !m.provides().impersonated_hosts.is_empty())
            .map(|m| m.id())
            .collect();
        for resolved in chosen.values_mut() {
            if emulating.contains(&resolved.provider)
                && let Connection::ObjectStore { tls_trust, .. } = &mut resolved.connection
            {
                *tls_trust = Some(Box::new(tls.trust.clone()));
            }
        }
    }
    Ok(())
}

/// Route a gateway-less environment: assign no routes and leave the [`GatewayConfig`] empty, so
/// every endpoint resolves directly, and publish each container's surface endpoints
/// ([`Api`](RouteIntent::Api), the UIs, [`Gatewayed`](RouteIntent::Gatewayed)) on a host port.
///
/// Each endpoint gets the path its service itself serves (its gateway prefix after the rewrite
/// the gateway would have applied), and one host port per `(service, internal port)`: the
/// endpoint's declared `host_port`, else its internal port plus
/// [`port_offset`](PlanCtx::port_offset), else, when another service already took that port,
/// the next from [`dedicated_listener_ports`](PlanCtx::dedicated_listener_ports) and then
/// [`dedicated_listener_port_base`](PlanCtx::dedicated_listener_port_base) upward.
///
/// A module that emulates public hostnames still needs them answered in-network, so the gateway
/// keeps only its TLS listener for them (the planner added it to the graph for that): it
/// publishes nothing and fronts no surface endpoint.
fn route_direct(settled: &mut Settled, ctx: &PlanCtx) -> Result<Routed, PlanError> {
    let Settled {
        graph,
        chosen,
        services: services_of,
        provisioned,
        ..
    } = settled;
    let mut gateway = GatewayConfig {
        admin_port: ctx.gateway_admin_port,
        ..GatewayConfig::default()
    };
    emulate_public_hosts(graph, provisioned, chosen, &mut gateway)?;

    // Host ports already declared on endpoints the gateway never fronts stay theirs.
    let mut taken: BTreeSet<u16> = services_of
        .values()
        .flatten()
        .flat_map(|s| &s.endpoints)
        .filter(|e| e.intent == RouteIntent::Internal)
        .filter_map(|e| e.host_port)
        .collect();
    let mut explicit = ctx.dedicated_listener_ports.iter().copied();
    let mut next_auto = ctx.dedicated_listener_port_base;
    let mut published: BTreeMap<ModuleId, Vec<PortMapping>> = BTreeMap::new();
    for module in &graph.nodes {
        let Some(services) = services_of.get_mut(module.id()) else {
            continue;
        };
        let mut mappings = Vec::new();
        for service in services
            .iter_mut()
            .filter(|s| matches!(s.placement, Placement::Container { .. }))
        {
            publish_direct(
                service,
                ctx.port_offset,
                &mut mappings,
                &mut taken,
                &mut || {
                    explicit.next().unwrap_or_else(|| {
                        let p = next_auto;
                        next_auto = next_auto.saturating_add(1);
                        p
                    })
                },
            );
        }
        if !mappings.is_empty() {
            published.insert(module.id().clone(), mappings);
        }
    }

    Ok(Routed {
        routes: RoutePlan::new(),
        gateway,
        facts: GatewayFacts {
            service: ctx.gateway_service.clone(),
            internal_port: ctx.gateway_internal_port,
            host_port: ctx.gateway_host_port,
            secure: None,
        },
        direct: Some(published),
    })
}

/// Give `service`'s surface endpoints their direct path and host port (see [`route_direct`]),
/// recording each newly published port in `mappings`. An endpoint without a declared host port
/// is published on its internal port shifted by `offset` (declared ones were shifted when the
/// services settled). `alloc` yields the next fallback port.
fn publish_direct(
    service: &mut ServiceSpec,
    offset: u16,
    mappings: &mut Vec<PortMapping>,
    taken: &mut BTreeSet<u16>,
    alloc: &mut dyn FnMut() -> u16,
) {
    // internal port → host port, so several endpoints on one port share one mapping.
    let mut host_of: BTreeMap<u16, u16> = BTreeMap::new();
    for endpoint in &mut service.endpoints {
        let served = match endpoint.intent {
            RouteIntent::Internal => continue,
            RouteIntent::Api => {
                let prefix = endpoint.mount_prefix.clone().unwrap_or_default();
                api_rewrite(&endpoint.rewrite, &service.base_path, &prefix).unwrap_or(prefix)
            }
            RouteIntent::UiPrefixable => service.base_path.clone(),
            RouteIntent::UiFixed | RouteIntent::Gatewayed => String::new(),
        };
        endpoint.path = join_path(&served, &endpoint.path);
        let host = *host_of.entry(endpoint.internal_port).or_insert_with(|| {
            let wanted = endpoint
                .host_port
                .unwrap_or_else(|| endpoint.internal_port.saturating_add(offset));
            let mut port = wanted;
            while taken.contains(&port) {
                port = alloc();
            }
            taken.insert(port);
            mappings.push(PortMapping {
                host: port,
                container: endpoint.internal_port,
            });
            port
        });
        endpoint.host_port = Some(host);
    }
}
