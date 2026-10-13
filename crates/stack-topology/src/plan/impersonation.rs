//! Emulated public hostnames: the gateway's TLS listener for modules that declare
//! [`impersonated_hosts`](crate::Provides::impersonated_hosts), and the rewrite of gatewayed
//! object-store connections onto the gateway origin.

use std::collections::BTreeMap;

use super::demands::ChosenProviders;
use super::gateway::{
    GATEWAY_CERTS_SERVICE, GATEWAY_TLS_PORT, GatewayConfig, TlsListenerConfig, TlsVirtualHost,
    ensure_named_cluster, gateway_tls_trust,
};
use super::resolve::ResolvedGraph;
use super::{PlanCtx, PlanError};
use crate::catalog::module::ModuleId;
use crate::model::connection::Connection;
use crate::model::endpoint::RouteIntent;
use crate::model::role::ServiceSpec;

/// Plan the gateway's TLS listener from every module's
/// [`impersonated_hosts`](crate::Provides::impersonated_hosts), or `None` if no module
/// declares any.
///
/// Walks modules in dependency order, expanding a `{name}` host once per resource the module
/// provisions, and adds a cluster per declared upstream. A host declared by two modules is a
/// [`PlanError::ImpersonatedHostCollision`].
pub(super) fn plan_impersonation(
    graph: &ResolvedGraph,
    provisioned_by: &BTreeMap<ModuleId, Vec<String>>,
    externals: &super::external::Externals,
    gateway: &mut GatewayConfig,
) -> Result<Option<TlsListenerConfig>, PlanError> {
    let mut owner: BTreeMap<String, ModuleId> = BTreeMap::new();
    let mut virtual_hosts: Vec<TlsVirtualHost> = Vec::new();
    // An external provider is the real cloud service: nothing to emulate.
    for module in graph.nodes.iter().filter(|m| !externals.contains(m.id())) {
        let objects = provisioned_by
            .get(module.id())
            .map(Vec::as_slice)
            .unwrap_or_default();
        for decl in &module.provides().impersonated_hosts {
            ensure_named_cluster(
                gateway,
                &decl.service,
                decl.service.clone(),
                decl.port,
                false,
            )?;
            let hosts: Vec<String> = if decl.host.contains("{name}") {
                objects
                    .iter()
                    .map(|name| decl.host.replace("{name}", name))
                    .collect()
            } else {
                vec![decl.host.clone()]
            };
            for host in hosts {
                if let Some(first) = owner.get(&host) {
                    return Err(PlanError::ImpersonatedHostCollision {
                        host,
                        first: first.clone(),
                        second: module.id().clone(),
                    });
                }
                owner.insert(host.clone(), module.id().clone());
                match virtual_hosts.iter_mut().find(|v| v.cluster == decl.service) {
                    Some(vhost) => vhost.hosts.push(host),
                    None => virtual_hosts.push(TlsVirtualHost {
                        cluster: decl.service.clone(),
                        hosts: vec![host],
                    }),
                }
            }
        }
    }
    if owner.is_empty() {
        return Ok(None);
    }
    Ok(Some(TlsListenerConfig {
        port: GATEWAY_TLS_PORT,
        virtual_hosts,
        hosts: owner.into_keys().collect(),
        trust: gateway_tls_trust(),
        mint_service: GATEWAY_CERTS_SERVICE.into(),
    }))
}

/// Rewrite every gatewayed object store's resolved connection so its `endpoint` (and the
/// `BlobEndpoint=` inside an Azure connection string) points at the gateway's dedicated
/// listener instead of the provider's in-network address.
///
/// An object-store provider advertises a [`Gatewayed`](RouteIntent::Gatewayed) endpoint;
/// `dedicated_ports` carries the listener host port assigned to it. For each such provider we
/// derive the in-network gateway origin `http://<gateway_service>:<port>` and swap it in for
/// the original origin in every `chosen` connection vended by that provider — keeping any path
/// the original endpoint carried (e.g. Azure's `/devstoreaccount1`). This is what makes
/// consumers (and the provider's own fragment) reach the store through Envoy.
pub(super) fn rewrite_gatewayed_object_store_endpoints(
    chosen: &mut ChosenProviders,
    services_of: &BTreeMap<ModuleId, Vec<ServiceSpec>>,
    graph: &ResolvedGraph,
    ctx: &PlanCtx,
    dedicated_ports: &BTreeMap<(String, String), u16>,
) {
    // provider module id → the in-network gateway origin its store is now reached at.
    let mut provider_origin: BTreeMap<ModuleId, String> = BTreeMap::new();
    for module in &graph.nodes {
        for service in &services_of[module.id()] {
            for endpoint in &service.endpoints {
                if endpoint.intent == RouteIntent::Gatewayed
                    && let Some(port) =
                        dedicated_ports.get(&(service.name.clone(), endpoint.id.clone()))
                {
                    provider_origin.insert(
                        module.id().clone(),
                        format!("http://{}:{}", ctx.gateway_service, port),
                    );
                }
            }
        }
    }
    if provider_origin.is_empty() {
        return;
    }
    for resolved in chosen.values_mut() {
        if let Some(new_origin) = provider_origin.get(&resolved.provider) {
            rewrite_object_store_origin(&mut resolved.connection, new_origin);
        }
    }
}

/// Swap the origin (`scheme://host:port`) of an object-store connection's `endpoint` — and the
/// `BlobEndpoint=` segment of an Azure connection string — to `new_origin`, preserving any
/// path. A no-op for non-object-store connections.
pub(super) fn rewrite_object_store_origin(connection: &mut Connection, new_origin: &str) {
    let Connection::ObjectStore {
        endpoint,
        credential,
        ..
    } = connection
    else {
        return;
    };
    *endpoint = swap_origin(endpoint, new_origin);
    if let crate::model::connection::ObjectStoreCredential::AzureBlob { connection_string } =
        credential
    {
        // The connection string embeds `BlobEndpoint=<url>;`; rewrite that URL's origin too.
        *connection_string = connection_string
            .split(';')
            .map(|seg| match seg.strip_prefix("BlobEndpoint=") {
                Some(url) => format!("BlobEndpoint={}", swap_origin(url, new_origin)),
                None => seg.to_string(),
            })
            .collect::<Vec<_>>()
            .join(";");
    }
}

/// Replace the `scheme://host[:port]` origin of `url` with `new_origin`, keeping the path.
/// `new_origin` is itself an origin (no trailing path). If `url` has no `://`, it is returned
/// unchanged.
pub(super) fn swap_origin(url: &str, new_origin: &str) -> String {
    match url.split_once("://") {
        Some((_scheme, rest)) => {
            // The path is everything from the first '/' after the authority.
            match rest.find('/') {
                Some(slash) => format!("{}{}", new_origin, &rest[slash..]),
                None => new_origin.to_string(),
            }
        }
        None => url.to_string(),
    }
}
