//! The gateway's structured config ([`GatewayConfig`]: listeners, clusters, forward-auth,
//! TLS) and the helpers the planner uses to claim prefixes and emit clusters onto it.

use std::collections::BTreeMap;

use super::knobs::{coerce_knob, resolve_knob};
use super::resolve::ResolvedGraph;
use super::{AppUpstream, PlanCtx, PlanError, Selection};
use crate::catalog::Catalog;
use crate::catalog::module::ModuleId;
use crate::model::endpoint::{Endpoint, Rewrite, RouteIntent, RouteOptions, Scheme};
use crate::model::placement::{HOST_GATEWAY_DNS, Placement};
use crate::model::role::{Role, ServiceSpec};

/// An upstream cluster the gateway forwards to — one per surface service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClusterConfig {
    /// The cluster's logical name (the service name).
    pub name: String,
    /// The upstream host (the service's compose DNS name).
    pub host: String,
    /// The upstream port (the endpoint's internal port).
    pub port: u16,
    /// Whether the upstream speaks HTTP/2 (a gRPC endpoint), so Envoy must too.
    pub http2: bool,
}

/// One route on a gateway listener.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GatewayRoute {
    /// The client-facing prefix the listener matches.
    pub prefix: String,
    /// The upstream cluster name to forward to.
    pub cluster: String,
    /// The upstream rewrite, if the path is changed before forwarding. `None`
    /// forwards the matched path unchanged (no rewrite emitted); `Some(path)`
    /// rewrites the matched prefix to `path`.
    pub rewrite: Option<String>,
    /// How the gateway treats requests on this route, carried from the fronted endpoint's
    /// [`Endpoint::route`](crate::Endpoint::route).
    pub options: RouteOptions,
}

/// A gateway listener: a host port and the routes it serves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListenerConfig {
    /// The host-published port this listener binds.
    pub host_port: u16,
    /// The listener's internal port inside the compose network.
    pub internal_port: u16,
    /// The routes on this listener, most-specific-first (Envoy match priority).
    pub routes: Vec<GatewayRoute>,
    /// Whether this listener is behind the gateway's sign-in when forward-auth is on. Always
    /// true for the shared listener; a dedicated one follows its endpoint's
    /// [`RouteOptions::gated`](crate::RouteOptions::gated).
    pub gated: bool,
}

/// The gateway knob key that turns on forward-auth. A bool knob with this key, declared on
/// the module filling the `gateway` role and resolving truthy, makes the planner pull in the
/// `auth`-role provider and gate the shared listener (see [`AuthConfig`]). The contract lives
/// here (the planner owns the wiring); the baseline catalog's `envoy` module declares the knob
/// under this same key.
pub const ENVOY_AUTH_KNOB: &str = "auth";

/// Legacy override key accepted as an alias of [`ENVOY_AUTH_KNOB`].
pub const ENVOY_AUTH_KNOB_LEGACY: &str = "ENVOY_AUTH";

/// The [`Provides::extras`](crate::Provides::extras) key by which the `auth`-role provider declares the HTTP `ext_authz`
/// endpoint path prefix the gateway posts authorization checks to. Provider-specific (Authelia
/// uses `/api/authz/ext-authz/`, other forward-auth proxies differ), so it lives in catalog data
/// rather than as a planner constant — keeping implementation names out of the planner.
pub const EXT_AUTHZ_PATH_EXTRA: &str = "ext_authz_path";

/// The trusted identity headers the gateway strips on ingress and re-adds from the auth
/// response — so the upstream always reads an auth-provider-asserted value, never a forged one.
/// These are the forward-auth convention (shared across providers, not implementation-specific),
/// so the planner supplies them as the default for any `auth`-role provider.
pub(super) const AUTH_IDENTITY_HEADERS: [&str; 4] = [
    "Remote-User",
    "Remote-Email",
    "Remote-Name",
    "Remote-Groups",
];

/// The `auth`-role provider the selection picks *explicitly* — by module id, or through the
/// `auth` capability (its first provider) — if any.
pub(super) fn selected_auth_provider(selection: &Selection, catalog: &Catalog) -> Option<ModuleId> {
    let providers = catalog.providers_of(Role::AUTH);
    selection
        .modules
        .iter()
        .find(|m| providers.contains(m))
        .cloned()
        .or_else(|| {
            selection
                .capabilities
                .iter()
                .any(|c| c == Role::AUTH)
                .then(|| providers.first().map(|p| (*p).clone()))
                .flatten()
        })
}

/// The `policy`-role provider the selection names (by module id or by the `policy`
/// capability), if any. Selecting one is what puts the gateway's authorization check in place.
pub(super) fn selected_policy_provider(
    selection: &Selection,
    catalog: &Catalog,
) -> Option<ModuleId> {
    let providers = catalog.providers_of(Role::POLICY);
    selection
        .modules
        .iter()
        .find(|m| providers.contains(m))
        .cloned()
        .or_else(|| {
            selection
                .capabilities
                .iter()
                .any(|c| c == Role::POLICY)
                .then(|| providers.first().map(|p| (*p).clone()))
                .flatten()
        })
}

/// A legacy `auth` override on a gateway module (the canonical [`ENVOY_AUTH_KNOB`] key or
/// its [`ENVOY_AUTH_KNOB_LEGACY`] alias), coerced to a bool. Read whether or not the gateway
/// still declares the knob, so manifests written before [`GatewaySettings`](super::GatewaySettings)
/// keep planning the same.
fn legacy_auth_override(
    selection: &Selection,
    gateway_id: &ModuleId,
) -> Result<Option<bool>, PlanError> {
    let Some(overrides) = selection.knob_overrides.get(gateway_id) else {
        return Ok(None);
    };
    let Some(raw) = overrides
        .get(ENVOY_AUTH_KNOB)
        .or_else(|| overrides.get(ENVOY_AUTH_KNOB_LEGACY))
    else {
        return Ok(None);
    };
    let kind = crate::catalog::module::KnobKind::Bool;
    match coerce_knob(raw, &kind) {
        Some(value) => Ok(Some(value == "true")),
        None => Err(PlanError::InvalidKnobValue {
            module: gateway_id.clone(),
            key: ENVOY_AUTH_KNOB.into(),
            value: raw.clone(),
            kind,
        }),
    }
}

/// Whether gateway forward-auth is on for this selection.
///
/// [`Selection::gateway`] wins when it asks for sign-in (directly or through `policy`).
/// Otherwise the legacy signals decide, read off whichever catalog module fills the `gateway`
/// role: an explicit `auth` knob override; else selecting an identity provider (see
/// [`selected_auth_provider`]) or a policy engine, since an auth provider the gateway never
/// consults would run as an orphan, up but authenticating nothing; else the gateway's own
/// `auth` knob default, if it declares one.
pub(super) fn gateway_auth_enabled(
    selection: &Selection,
    catalog: &Catalog,
) -> Result<bool, PlanError> {
    if selection.gateway.auth_enabled() {
        return Ok(true);
    }
    for gateway_id in catalog.providers_of(Role::GATEWAY) {
        if let Some(explicit) = legacy_auth_override(selection, gateway_id)? {
            return Ok(explicit);
        }
    }
    if selected_auth_provider(selection, catalog).is_some()
        || selected_policy_provider(selection, catalog).is_some()
    {
        return Ok(true);
    }
    for gateway_id in catalog.providers_of(Role::GATEWAY) {
        let Some(module) = catalog.get(gateway_id) else {
            continue;
        };
        if let Some(knob) = module.knobs().iter().find(|k| k.key == ENVOY_AUTH_KNOB)
            && let Some(value) = resolve_knob(gateway_id, knob, None)?
        {
            return Ok(value == "true");
        }
    }
    Ok(false)
}

/// The `policy`-role provider to wire: the one the selection names, else the catalog's first
/// when [`GatewaySettings::policy`](super::GatewaySettings::policy) asks for one.
pub(super) fn policy_provider(selection: &Selection, catalog: &Catalog) -> Option<ModuleId> {
    selected_policy_provider(selection, catalog).or_else(|| {
        selection
            .gateway
            .policy
            .then(|| {
                catalog
                    .providers_of(Role::POLICY)
                    .first()
                    .map(|p| (*p).clone())
            })
            .flatten()
    })
}

/// The `gateway`-role provider an environment runs: the one the selection names, else
/// the first preferred via [`PlanCtx::provider_preference`], else the catalog's first. `None`
/// only for a catalog without a gateway.
pub(super) fn gateway_provider(
    selection: &Selection,
    catalog: &Catalog,
    ctx: &PlanCtx,
) -> Option<ModuleId> {
    let providers = catalog.providers_of(Role::GATEWAY);
    selection
        .modules
        .iter()
        .find(|m| providers.contains(m))
        .or_else(|| {
            ctx.provider_preference
                .get(Role::GATEWAY)
                .into_iter()
                .flatten()
                .find(|m| providers.contains(m))
        })
        .or_else(|| providers.first().copied())
        .cloned()
}

/// Forward-auth (single-sign-on) configuration for the gateway: present only when the
/// gateway's sign-in is on. It tells the Envoy renderer to gate the shared
/// listener (API + UI routes) behind an `ext_authz` HTTP filter pointed at the auth
/// provider, and which identity headers to treat as trusted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthConfig {
    /// The upstream cluster the `ext_authz` filter calls (an entry in
    /// [`GatewayConfig::clusters`]). The bundled provider is `authelia`.
    pub cluster: String,
    /// The auth provider's port (the `server_uri` authority for the `ext_authz` filter).
    pub port: u16,
    /// The auth endpoint path prefix the filter posts to (Authelia's
    /// `/api/authz/ext-authz/`).
    pub path_prefix: String,
    /// The client-facing path prefix the auth provider's **login portal** is served at, routed
    /// through the gateway to the provider's cluster with `ext_authz` disabled on that route (so
    /// a logged-out user can actually reach the login page the deny-redirect sends them to). The
    /// provider's `authelia_url` / session config must point here. Defaults to `/authelia`.
    pub portal_prefix: String,
    /// The trusted identity headers the gateway both **strips on ingress** (so a client
    /// cannot forge them) and **allows upstream** from the auth response (so the app reads
    /// an auth-provider-asserted value). E.g. `Remote-User` / `Remote-Email` / `Remote-Name`
    /// / `Remote-Groups`.
    pub identity_headers: Vec<String>,
}

/// The structured gateway configuration the planner emits — what a consumer turns
/// into an Envoy (or other) config, replacing hand-authored route/cluster lists.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GatewayConfig {
    /// The listeners (the shared one plus any dedicated for fixed-path UIs).
    pub listeners: Vec<ListenerConfig>,
    /// The upstream clusters, deduplicated by name.
    pub clusters: Vec<ClusterConfig>,
    /// The Envoy admin port (bound on both sides). Set from
    /// [`PlanCtx::gateway_admin_port`]; the `Default` of `0` is only ever a transient
    /// accumulator value the planner overwrites.
    pub admin_port: u16,
    /// Forward-auth configuration, present only when gateway sign-in is on.
    /// When set, the Envoy renderer gates the shared listener behind `ext_authz`.
    pub auth: Option<AuthConfig>,
    /// The TLS listener emulating public hostnames, present only when some selected module
    /// declares [`impersonated_hosts`](crate::Provides::impersonated_hosts).
    pub tls: Option<TlsListenerConfig>,
    /// The authorization policy engine, present only when a `policy`-role module is selected.
    /// When set, the Envoy renderer adds a gRPC `ext_authz` check *after* forward-auth on the
    /// shared listener, so the policy decides on the identity the auth provider asserted.
    pub policy: Option<PolicyConfig>,
    /// The shared listener's HTTPS origin, present exactly when [`auth`](Self::auth) is.
    pub secure: Option<SecureOrigin>,
}

/// The shared listener's origin under forward-auth (see [`GatewayConfig::secure`]).
///
/// The auth provider only issues sessions for a secure origin under a dotted cookie domain, so
/// bare `http://localhost` cannot sign anyone in. With auth on, the host side reaches the shared
/// listener at `https://<public_host>:<port>` (TLS with the gateway's minted CA), and the
/// listener keeps accepting plain HTTP from in-network callers, who address it by the dotted
/// [`internal_host`](Self::internal_host) network alias.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecureOrigin {
    /// The host-side hostname, `<env>.localhost`: browsers and most clients resolve every
    /// `*.localhost` name to the loopback address, so it needs no `/etc/hosts` entry.
    pub public_host: String,
    /// The in-network alias, `<env>.internal`. Not a `*.localhost` name, because some clients
    /// (curl among them) resolve those to loopback without asking DNS.
    pub internal_host: String,
    /// Where the CA and the listener's certificate live.
    pub trust: crate::model::connection::TlsTrust,
    /// The compose one-shot service that mints the certificate.
    pub mint_service: String,
}

impl SecureOrigin {
    /// The secure origin for an environment named `env_name`.
    pub(super) fn for_env(env_name: &str) -> Self {
        let label = dns_label(env_name);
        SecureOrigin {
            public_host: format!("{label}.localhost"),
            internal_host: format!("{label}.internal"),
            trust: gateway_tls_trust(),
            mint_service: GATEWAY_CERTS_SERVICE.into(),
        }
    }
}

/// `name` as a DNS label: lowercased, with every character outside `[a-z0-9-]` mapped to `-`,
/// no leading or trailing `-`, and at most 63 characters. Falls back to `trestle` when nothing
/// usable remains.
pub(super) fn dns_label(name: &str) -> String {
    let mapped: String = name
        .chars()
        .map(|c| match c.to_ascii_lowercase() {
            c @ ('a'..='z' | '0'..='9') => c,
            _ => '-',
        })
        .collect();
    let mut label = mapped.trim_matches('-').to_string();
    label.truncate(63);
    let label = label.trim_end_matches('-');
    if label.is_empty() {
        "trestle".into()
    } else {
        label.into()
    }
}

/// The gateway's authorization-policy wiring (see [`GatewayConfig::policy`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyConfig {
    /// The upstream (HTTP/2) cluster the gRPC `ext_authz` check calls.
    pub cluster: String,
}

/// The gateway's in-network TLS listener for emulated public hostnames.
///
/// Internal only: it is never host-published. In-network clients reach it because the
/// gateway holds every [`hosts`](Self::hosts) entry as a compose network alias; host-side
/// clients keep using the plain dedicated listeners via an endpoint override.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TlsListenerConfig {
    /// The port the listener binds inside the compose network ([`GATEWAY_TLS_PORT`]).
    pub port: u16,
    /// One virtual host per upstream cluster, in dependency order.
    pub virtual_hosts: Vec<TlsVirtualHost>,
    /// Every emulated hostname, sorted and deduplicated.
    pub hosts: Vec<String>,
    /// Where the CA and the listener's certificate live.
    pub trust: crate::model::connection::TlsTrust,
    /// The compose one-shot service that mints the certificates.
    pub mint_service: String,
}

/// The hostnames one upstream cluster answers on the [`TlsListenerConfig`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TlsVirtualHost {
    /// The upstream cluster (an entry in [`GatewayConfig::clusters`]).
    pub cluster: String,
    /// The exact hostnames routed to [`cluster`](Self::cluster), without ports.
    pub hosts: Vec<String>,
    /// The [`response_headers`](crate::ImpersonatedHost::response_headers) of every host
    /// routed here, merged.
    pub response_headers: BTreeMap<String, String>,
}

/// The in-network port the gateway's TLS listener binds (see [`TlsListenerConfig`]). Fixed at
/// the HTTPS default because emulated clients call the bare public hostname.
pub const GATEWAY_TLS_PORT: u16 = 443;

/// The compose named volume the gateway's CA material lives in.
pub(super) const GATEWAY_CERTS_VOLUME: &str = "gateway-certs";

/// Where the gateway and its consumers mount [`GATEWAY_CERTS_VOLUME`].
pub(super) const GATEWAY_CERTS_MOUNT: &str = "/certs";

/// The one-shot service that mints the gateway's CA material, defined by the gateway
/// module's fragment.
pub(super) const GATEWAY_CERTS_SERVICE: &str = "gateway-certs";

/// The [`TlsTrust`](crate::TlsTrust) for the gateway's minted CA.
pub(super) fn gateway_tls_trust() -> crate::model::connection::TlsTrust {
    crate::model::connection::TlsTrust {
        volume: GATEWAY_CERTS_VOLUME.into(),
        mount_path: GATEWAY_CERTS_MOUNT.into(),
        ca_pem: format!("{GATEWAY_CERTS_MOUNT}/ca.pem"),
        jvm_truststore: format!("{GATEWAY_CERTS_MOUNT}/truststore.p12"),
        // The JDK's default truststore password; the truststore holds only public certs.
        jvm_truststore_password: "changeit".into(),
    }
}

/// Claim `prefix` for `service.endpoint`, erroring if another endpoint already did.
pub(super) fn claim(
    claimed: &mut BTreeMap<String, String>,
    prefix: &str,
    service: &ServiceSpec,
    endpoint_id: &str,
) -> Result<(), PlanError> {
    let who = format!("{}.{}", service.name, endpoint_id);
    if let Some(first) = claimed.get(prefix) {
        return Err(PlanError::PrefixCollision {
            prefix: prefix.to_string(),
            first: first.clone(),
            second: who,
        });
    }
    claimed.insert(prefix.to_string(), who);
    Ok(())
}

/// Reject an empty mount prefix — it would become a `/` catch-all that shadows every
/// other route.
pub(super) fn require_prefix(
    prefix: &str,
    service: &ServiceSpec,
    endpoint_id: &str,
) -> Result<(), PlanError> {
    if prefix.is_empty() {
        return Err(PlanError::MissingPrefix(format!(
            "{}.{}",
            service.name, endpoint_id
        )));
    }
    Ok(())
}

/// Ensure the gateway has a cluster for `service`'s `endpoint` and return its name.
///
/// A service's first gateway-facing port gets the service-named cluster; an endpoint on a
/// *different* port of the same service (e.g. a UI and an ingest port) gets its own
/// `<service>_<endpoint>` cluster, so one service can expose several ports at the gateway.
pub(super) fn ensure_cluster(
    gateway: &mut GatewayConfig,
    service: &ServiceSpec,
    endpoint: &Endpoint,
) -> Result<String, PlanError> {
    let host = match &service.placement {
        Placement::Container { service } => service.clone(),
        // Host/in-process services are reached via the host gateway DNS from a
        // container; the gateway cluster targets that.
        _ => HOST_GATEWAY_DNS.to_string(),
    };
    let port = endpoint.internal_port;
    let name = match gateway.clusters.iter().find(|c| c.name == service.name) {
        Some(existing) if existing.port != port => format!("{}_{}", service.name, endpoint.id),
        _ => service.name.clone(),
    };
    ensure_named_cluster(gateway, &name, host, port, endpoint.scheme == Scheme::Grpc)?;
    Ok(name)
}

/// Add the cluster `name` → `host:port` unless it already exists. An existing cluster of the
/// same name on a different port is a [`PlanError::ClusterPortConflict`].
pub(super) fn ensure_named_cluster(
    gateway: &mut GatewayConfig,
    name: &str,
    host: String,
    port: u16,
    http2: bool,
) -> Result<(), PlanError> {
    if let Some(existing) = gateway.clusters.iter().find(|c| c.name == name) {
        if existing.port != port {
            return Err(PlanError::ClusterPortConflict {
                service: name.to_string(),
                first: existing.port,
                second: port,
            });
        }
        return Ok(());
    }
    gateway.clusters.push(ClusterConfig {
        name: name.to_string(),
        host,
        port,
        http2,
    });
    Ok(())
}

/// The gateway rewrite for an API route, for the structured [`GatewayConfig`], from the
/// endpoint's typed [`Rewrite`] and its service's `base_path`.
///
/// Tri-state result: `None` means forward the path unchanged (no rewrite emitted);
/// `Some(path)` means rewrite the matched prefix to `path`.
///
/// - [`Rewrite::Passthrough`] forwards unchanged (`None`).
/// - [`Rewrite::To`] rewrites to the literal path.
/// - [`Rewrite::Inherit`] joins the service's `base_path` with the client `prefix`; a
///   service serving at root (empty base path) needs no rewrite (`None`).
pub(super) fn api_rewrite(rewrite: &Rewrite, base_path: &str, prefix: &str) -> Option<String> {
    match rewrite {
        Rewrite::Passthrough => None,
        Rewrite::To(path) => Some(path.clone()),
        Rewrite::Inherit => {
            if base_path.is_empty() {
                None
            } else {
                Some(join_path(base_path, prefix))
            }
        }
    }
}

/// Join two path segments with exactly one slash at the seam, no trailing slash
/// duplication.
pub(super) fn join_path(a: &str, b: &str) -> String {
    let a = a.trim_end_matches('/');
    let b = b.trim_start_matches('/');
    if b.is_empty() {
        a.to_string()
    } else if a.is_empty() {
        format!("/{b}")
    } else {
        format!("{a}/{b}")
    }
}

/// Allocate a dedicated listener port, in graph order, for every endpoint that needs its own
/// listener ([`UiFixed`](RouteIntent::UiFixed) / [`Gatewayed`](RouteIntent::Gatewayed)), keyed
/// by `(service, endpoint)`.
///
/// Ports come from the explicit [`dedicated_listener_ports`](PlanCtx::dedicated_listener_ports)
/// first, then auto-allocate upward from
/// [`dedicated_listener_port_base`](PlanCtx::dedicated_listener_port_base). Done before the
/// main routing loop: a provider is ordered before its consumers, but a consumer binds its
/// connection earlier in its own iteration than the loop would reach the provider's route.
pub(super) fn allocate_dedicated_ports(
    graph: &ResolvedGraph,
    services_of: &BTreeMap<ModuleId, Vec<ServiceSpec>>,
    ctx: &PlanCtx,
) -> BTreeMap<(String, String), u16> {
    let mut explicit = ctx.dedicated_listener_ports.iter().copied();
    let mut next_auto = ctx.dedicated_listener_port_base;
    let mut alloc = || {
        explicit.next().unwrap_or_else(|| {
            let p = next_auto;
            next_auto = next_auto.saturating_add(1);
            p
        })
    };
    let mut ports = BTreeMap::new();
    for module in &graph.nodes {
        for service in &services_of[module.id()] {
            for endpoint in &service.endpoints {
                if matches!(
                    endpoint.intent,
                    RouteIntent::UiFixed | RouteIntent::Gatewayed
                ) {
                    ports.insert((service.name.clone(), endpoint.id.clone()), alloc());
                }
            }
        }
    }
    ports
}

/// Make the user's app the gateway's catch-all: an `app` cluster and a `/` route on the shared
/// listener, after the same collision checks every module route gets — so an app-vs-module
/// conflict fails here rather than as an Envoy config error at launch.
pub(super) fn wire_app_upstream(
    app: &AppUpstream,
    claimed: &BTreeMap<String, String>,
    gateway: &mut GatewayConfig,
    shared_routes: &mut Vec<GatewayRoute>,
) -> Result<(), PlanError> {
    // A module that already claimed `/` (an Api `mount_prefix: "/"` or a UiPrefixable
    // `base_path: "/"`) would be silently shadowed by the catch-all; reject instead.
    if let Some(first) = claimed.get("/") {
        return Err(PlanError::AppUpstreamCollision {
            conflict: first.clone(),
            reason: "it already claims the `/` route the app catch-all needs".into(),
        });
    }
    // `app` is the reserved cluster name for the upstream; a module service named `app`
    // would produce a duplicate Envoy cluster.
    if let Some(c) = gateway.clusters.iter().find(|c| c.name == "app") {
        return Err(PlanError::AppUpstreamCollision {
            conflict: c.name.clone(),
            reason: "a module contributes a service named `app`, the cluster name the app \
                     upstream reserves"
                .into(),
        });
    }
    gateway.clusters.push(ClusterConfig {
        name: "app".into(),
        host: app.service.clone(),
        port: app.port,
        http2: false,
    });
    shared_routes.push(GatewayRoute {
        prefix: "/".into(),
        cluster: "app".into(),
        rewrite: None,
        options: RouteOptions::default(),
    });
    Ok(())
}

/// Wire forward-auth onto the gateway for the resolved auth `provider`: its upstream cluster
/// plus the [`AuthConfig`] that gates the shared listener behind `ext_authz`.
///
/// The cluster name/host/port come from the provider's resolved service and `Internal`
/// endpoint — not hardcoded constants — so a differently-named or -ported provider is wired
/// correctly; the `ext_authz` path is the provider's declared [`EXT_AUTHZ_PATH_EXTRA`].
pub(super) fn wire_auth(
    provider: &ModuleId,
    services_of: &BTreeMap<ModuleId, Vec<ServiceSpec>>,
    catalog: &Catalog,
    gateway: &mut GatewayConfig,
) -> Result<(), PlanError> {
    let service =
        services_of[provider]
            .first()
            .ok_or_else(|| PlanError::AuthProviderMisconfigured {
                module: provider.clone(),
                reason: "declares no service".into(),
            })?;
    let endpoint = service
        .endpoints
        .iter()
        .find(|e| e.intent == RouteIntent::Internal)
        .ok_or_else(|| PlanError::AuthProviderMisconfigured {
            module: provider.clone(),
            reason: "service has no Internal endpoint".into(),
        })?;
    let path_prefix = catalog
        .get(provider)
        .and_then(|m| m.provides().extras.get(EXT_AUTHZ_PATH_EXTRA))
        .ok_or_else(|| PlanError::AuthProviderMisconfigured {
            module: provider.clone(),
            reason: format!("does not declare the `{EXT_AUTHZ_PATH_EXTRA}` extra"),
        })?
        .clone();
    let cluster = ensure_cluster(gateway, service, endpoint)?;
    // The login portal is routed through the gateway at `/<service>` (a generic, provider-
    // agnostic path — `/authelia` for the bundled provider), so the deny-redirect a
    // logged-out browser follows actually resolves. The provider's session config must point
    // its `authelia_url` here.
    let portal_prefix = format!("/{}", service.name);
    gateway.auth = Some(AuthConfig {
        cluster,
        port: endpoint.internal_port,
        path_prefix,
        portal_prefix,
        identity_headers: AUTH_IDENTITY_HEADERS
            .iter()
            .map(|h| (*h).to_string())
            .collect(),
    });
    Ok(())
}

/// Wire the authorization policy engine onto the gateway: an HTTP/2 cluster for the provider's
/// gRPC [`Internal`](RouteIntent::Internal) endpoint, and the [`PolicyConfig`] the Envoy
/// renderer turns into a gRPC `ext_authz` check after forward-auth.
pub(super) fn wire_policy(
    provider: &ModuleId,
    services_of: &BTreeMap<ModuleId, Vec<ServiceSpec>>,
    gateway: &mut GatewayConfig,
) -> Result<(), PlanError> {
    let misconfigured = |reason: &str| PlanError::PolicyProviderMisconfigured {
        module: provider.clone(),
        reason: reason.into(),
    };
    let service = services_of[provider]
        .first()
        .ok_or_else(|| misconfigured("declares no service"))?;
    let endpoint = service
        .endpoints
        .iter()
        .find(|e| e.intent == RouteIntent::Internal && e.scheme == Scheme::Grpc)
        .ok_or_else(|| misconfigured("service has no Internal gRPC endpoint"))?;
    let cluster = ensure_cluster(gateway, service, endpoint)?;
    gateway.policy = Some(PolicyConfig { cluster });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_names_become_dns_labels() {
        for (name, label) in [
            ("demo", "demo"),
            ("My_Stack", "my-stack"),
            ("-edge-", "edge"),
            ("___", "trestle"),
            ("", "trestle"),
        ] {
            assert_eq!(dns_label(name), label, "{name:?}");
        }
        assert_eq!(dns_label(&"a".repeat(80)).len(), 63);
        assert_eq!(dns_label(&format!("{}-b", "a".repeat(62))), "a".repeat(62));
    }

    #[test]
    fn secure_origin_hosts_follow_the_env_name() {
        let origin = SecureOrigin::for_env("demo");
        assert_eq!(origin.public_host, "demo.localhost");
        assert_eq!(origin.internal_host, "demo.internal");
        assert_eq!(origin.mint_service, GATEWAY_CERTS_SERVICE);
    }
}
