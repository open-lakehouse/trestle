//! Phase 3 — **route**: assign every surface endpoint a non-colliding gateway route and build
//! the structured [`GatewayConfig`] (listeners, clusters, forward-auth, authorization, TLS).
//!
//! This is the one phase that sees every module's endpoints at once, so it is where prefixes
//! are decided. It also finalizes the object-store connections the settle phase resolved: a
//! gatewayed store's endpoint is rewritten to the gateway, and a store whose cloud hostnames the
//! gateway emulates carries the trust for the gateway's local CA.

use std::collections::{BTreeMap, BTreeSet};

use super::gateway::{
    allocate_dedicated_ports, api_rewrite, claim, ensure_cluster, require_prefix,
    wire_app_upstream, wire_auth, wire_policy,
};
use super::impersonation::{plan_impersonation, rewrite_gatewayed_object_store_endpoints};
use super::routing::{AssignedRoute, Listener, RoutePlan};
use super::settle::Settled;
use super::{
    GatewayConfig, GatewayFacts, GatewayRoute, ListenerConfig, PlanCtx, PlanError, SecureOrigin,
};
use crate::catalog::Catalog;
use crate::catalog::module::ModuleId;
use crate::model::connection::Connection;
use crate::model::endpoint::RouteIntent;

/// The routing decisions: the per-endpoint routes the address resolver reads, the gateway
/// config the Envoy renderer reads, and the gateway facts both resolve addresses against.
pub(super) struct Routed {
    /// The per-endpoint route assignments.
    pub routes: RoutePlan,
    /// The structured gateway config.
    pub gateway: GatewayConfig,
    /// The gateway's name, ports and (under forward-auth) secure origin.
    pub facts: GatewayFacts,
}

/// Run the route phase (see the [module docs](self)), rewriting `settled.chosen`'s gatewayed
/// and emulated object-store connections in place.
pub(super) fn route(
    settled: &mut Settled,
    catalog: &Catalog,
    ctx: &PlanCtx,
    auth_provider: Option<&ModuleId>,
    policy_provider: Option<&ModuleId>,
) -> Result<Routed, PlanError> {
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
    // Emulated public hostnames: give the gateway a TLS listener for them, and mark every
    // connection those providers vend with the trust a consumer needs for the local CA.
    gateway.tls = plan_impersonation(graph, provisioned, &mut gateway)?;
    // Forward-auth needs a secure, dotted origin (see `SecureOrigin`).
    if stack_facts.gateway_auth {
        gateway.secure = Some(SecureOrigin::for_env(&ctx.env_name));
    }
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
    })
}
