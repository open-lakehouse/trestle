//! The **egress gateway**: the Envoy the environment's services reach upstreams outside it
//! through, which adds each route's credential to every request it forwards. Its secrets come
//! from the environment's secret server over SDS, so no value is written to disk.
//!
//! Every route gets its own listener, on internal ports from [`EGRESS_FIRST_PORT`] in route
//! order: Envoy's `credential_injector` has no per-route configuration, so each credential needs
//! its own filter chain. The ports never reach the host. A consumer calls
//! `http://egress-gateway:<port><path>`, which it reads from `EGRESS_<ROUTE>_URL` (see
//! [`egress_url_var`]).
//!
//! A route comes from the selection ([`Selection::egress`](super::Selection::egress)) or from a
//! module that declares an outside endpoint with an
//! [injected credential](crate::Endpoint::inject_credential): that module is the route's one
//! consumer.
//!
//! A route's secret is declared on the egress module alone (the secret server reads it), so no
//! other service's rendered fragment names it.
//!
//! The secret server has **consumers**, one SDS socket each ([`SdsConsumer`]): the egress
//! Envoy, which asks for the routes' secrets, and every module that reads
//! [secrets over SDS](crate::Provides::sds_secrets) or seals under
//! [keys](crate::Provides::keys) through its key service. A module's SDS secrets move to the
//! egress module in the same way a route's do, so they too reach the secret server alone. A
//! key's keyring is held by a variable the planner derives ([`keyring_var`]) and declares on the
//! egress module only: the module never sees it.
//!
//! The secret server reads those variables from its launch environment, or, with the
//! [host source](crate::SecretSource::Host), from a host endpoint the launcher runs
//! ([`HostSource`]): then its only launch value is [`HOST_SOURCE_TOKEN_VAR`], the instance's
//! path token on that endpoint, and the variables stay declared (the launcher resolves them for
//! the endpoint) but reach no container.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use super::hosted::{Secrets, env_refs, function_url_var};
use super::{PlanError, Selection};
use crate::catalog::Catalog;
use crate::catalog::module::{DeclaredSecret, KeyImport, ModuleId, SdsSecret, StackFacts};
use crate::model::egress::{EgressRoute, McpRoute, egress_url_var};
use crate::model::endpoint::InjectedCredential;
use crate::model::placement::Placement;
use crate::model::role::{Role, ServiceSpec};

/// The first route's listener port; each next route takes the next port.
pub const EGRESS_FIRST_PORT: u16 = 10001;
/// The egress Envoy's admin port, bound to the container's loopback only.
pub const EGRESS_ADMIN_PORT: u16 = 9901;
/// The egress Envoy's compose service, and the host consumers reach it at.
pub const EGRESS_SERVICE: &str = "egress-gateway";
/// The secret server's compose service.
pub const SECRET_SERVER_SERVICE: &str = "secret-server";
/// Where the SDS sockets live: the server mounts each consumer's volume at
/// `<dir>/<consumer>/`, the consumer at `<dir>/`.
pub const SDS_DIR: &str = "/run/trestle-sds";
/// The variable a module reading secrets over SDS finds its socket's path in.
pub const SDS_SOCKET_VAR: &str = "TRESTLE_SDS_SOCKET";
/// The socket's file name in an SDS volume.
const SDS_SOCKET: &str = "sds.sock";
/// The secret server's launch variable for its host endpoint's path token, with the
/// [host source](crate::SecretSource::Host): its only launch value.
pub const HOST_SOURCE_TOKEN_VAR: &str = "SECRET_SERVER_HOST_TOKEN";
/// The name the secret server reaches the host at. Docker Desktop, OrbStack and Colima resolve
/// it to the host's loopback; on a Linux engine the service maps it to the bridge's gateway
/// (`extra_hosts: host-gateway`).
pub const HOST_SOURCE_HOST: &str = "host.docker.internal";
/// The longest key name the secret server accepts (a KEK id `<key>@v<n>` must fit in 255
/// bytes).
pub const MAX_KEY_NAME_LEN: usize = 243;

/// The variable holding the keyring of `module`'s key `key`, which the secret server alone
/// reads: `TRESTLE_KEYRING_<MODULE>__<KEY>`, each in upper snake case.
pub fn keyring_var(module: &ModuleId, key: &str) -> String {
    format!(
        "TRESTLE_KEYRING_{}__{}",
        upper_snake(module.as_str()),
        upper_snake(key)
    )
}

/// The variable a module finds the name of its key `key` in: `TRESTLE_KEY_<KEY>`, in upper
/// snake case.
pub fn key_name_var(key: &str) -> String {
    format!("TRESTLE_KEY_{}", upper_snake(key))
}

fn upper_snake(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '-' => '_',
            c => c.to_ascii_uppercase(),
        })
        .collect()
}

/// The egress gateway the plan runs (see the [module docs](self)): the module's render reads it
/// as the `egress` global.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EgressConfig {
    /// The egress module.
    pub module: ModuleId,
    /// The routes, in listener order.
    pub routes: Vec<PlannedEgressRoute>,
    /// Every cluster outside the environment the routes reach: each route's upstream, then its
    /// OAuth2 token endpoint, in route order.
    pub clusters: Vec<EgressUpstream>,
    /// The variables the secret server serves, once each: the routes' secrets in route order,
    /// then the SDS consumers' in consumer order. It reads them from its launch environment, or
    /// with [`host`](Self::host), from the host endpoint.
    pub secrets: Vec<String>,
    /// The host endpoint the secret server takes its values from, with the
    /// [host source](crate::SecretSource::Host). `None`: its launch environment.
    pub host: Option<HostSource>,
    /// The secret server's consumers: one SDS socket, and volume, each.
    pub sds_consumers: Vec<SdsConsumer>,
    /// The Envoy admin port (loopback only, never published).
    pub admin_port: u16,
    /// The variables the routes' upstreams still read: a function's URL a plan without secrets
    /// couldn't resolve. Envoy can't read a variable, so the gateway refuses to start until a
    /// render resolves them.
    pub unresolved_upstreams: Vec<String>,
}

/// The host endpoint of the [host source](crate::SecretSource::Host).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HostSource {
    /// The port the endpoint listens on, on the host.
    pub port: u16,
    /// The base URL the secret server reaches it at: `http://host.docker.internal:<port>`.
    pub url: String,
    /// The launch variable holding the instance's path token ([`HOST_SOURCE_TOKEN_VAR`]).
    pub token_env: String,
}

impl EgressConfig {
    /// The variables the secret server's container is launched with: the token alone with a
    /// host source, else every variable it serves.
    pub fn launch_vars(&self) -> Vec<&str> {
        match &self.host {
            Some(host) => vec![host.token_env.as_str()],
            None => self.secrets.iter().map(String::as_str).collect(),
        }
    }

    /// The routes' secrets, as the egress module declares them: each one required.
    pub(super) fn declared_route_secrets(&self) -> Vec<DeclaredSecret> {
        let mut vars: Vec<&str> = Vec::new();
        for var in self.routes.iter().map(|r| r.credential.secret_var()) {
            if !vars.contains(&var) {
                vars.push(var);
            }
        }
        vars.into_iter()
            .map(|var| {
                let routes: Vec<&str> = self
                    .routes
                    .iter()
                    .filter(|r| r.credential.secret_var() == var)
                    .map(|r| r.name.as_str())
                    .collect();
                DeclaredSecret::required(
                    var,
                    format!(
                        "The credential the egress gateway adds to requests on route `{}`.",
                        routes.join("`, `")
                    ),
                )
            })
            .collect()
    }

    /// The route URLs `consumer` gets: `EGRESS_<ROUTE>_URL` → `http://egress-gateway:<port><path>`.
    pub fn consumer_env(&self, consumer: &ModuleId) -> BTreeMap<String, String> {
        self.routes
            .iter()
            .filter(|r| r.consumers.contains(consumer))
            .map(|r| (r.url_var.clone(), r.url.clone()))
            .collect()
    }

    /// The routes `consumer` may call, in listener order, as it sees them: where to call each
    /// and what it serves, never its credential.
    pub fn consumed_by(&self, consumer: &ModuleId) -> Vec<ConsumedEgressRoute> {
        self.routes
            .iter()
            .filter(|r| r.consumers.contains(consumer))
            .map(|r| ConsumedEgressRoute {
                name: r.name.clone(),
                url: r.url.clone(),
                url_var: r.url_var.clone(),
                mcp: r.mcp.clone(),
            })
            .collect()
    }

    /// The SDS consumer that is a module (not the egress Envoy), if `module` reads secrets over
    /// SDS.
    pub fn sds_consumer(&self, module: &ModuleId) -> Option<&SdsConsumer> {
        self.sds_consumers
            .iter()
            .find(|c| c.module.as_ref() == Some(module))
    }

    /// Every key the secret server holds a keyring for: of each module consumer, in consumer
    /// order.
    pub fn keys(&self) -> impl Iterator<Item = (&ModuleId, &ServedKey)> {
        self.sds_consumers
            .iter()
            .filter_map(|c| c.module.as_ref().map(|m| (m, &c.keys)))
            .flat_map(|(m, keys)| keys.iter().map(move |k| (m, k)))
    }

    /// Every module that calls some route, in route order.
    pub fn consumers(&self) -> Vec<&ModuleId> {
        let mut out: Vec<&ModuleId> = Vec::new();
        for consumer in self.routes.iter().flat_map(|r| &r.consumers) {
            if !out.contains(&consumer) {
                out.push(consumer);
            }
        }
        out
    }
}

/// One route through the egress gateway, as planned.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PlannedEgressRoute {
    /// The route's name: its secret's name on the secret server.
    pub name: String,
    /// The listener's port.
    pub port: u16,
    /// The path prefix it forwards, unchanged.
    pub path: String,
    /// What consumers call: `http://egress-gateway:<port><path>`.
    pub url: String,
    /// The variable consumers read [`url`](Self::url) from.
    pub url_var: String,
    /// The Neon Function the route reaches (its slug), for a function route.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<String>,
    /// The upstream's origin: a URL, or a `${VAR}` the plan couldn't resolve.
    pub origin: String,
    /// The Envoy cluster the route forwards to.
    pub upstream: EgressUpstream,
    /// The credential added to every request.
    pub credential: InjectedCredential,
    /// The OAuth2 token endpoint's cluster, for client-credentials routes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_endpoint: Option<TokenEndpoint>,
    /// The modules that may call it.
    pub consumers: Vec<ModuleId>,
    /// What it serves, when it is an MCP server.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp: Option<McpRoute>,
}

/// A route as one of its consumers sees it (see [`EgressConfig::consumed_by`]): a consumer's
/// render reads these as the `egress_routes` global, to configure itself (an MCP registry, say).
/// It carries nothing about the credential, so no consumer's files can name the route's secret.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ConsumedEgressRoute {
    /// The route's name.
    pub name: String,
    /// What the consumer calls: `http://egress-gateway:<port><path>`.
    pub url: String,
    /// The variable the consumer reads [`url`](Self::url) from.
    pub url_var: String,
    /// What it serves, when it is an MCP server.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp: Option<McpRoute>,
}

/// An Envoy cluster outside the environment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EgressUpstream {
    /// The cluster's name.
    pub cluster: String,
    /// The host: a DNS name, or a `${VAR}` not resolved yet.
    pub host: String,
    /// The port.
    pub port: u16,
    /// The TLS server name (SNI and the certificate's DNS name) for an `https` upstream.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls: Option<String>,
}

/// Where an OAuth2 client-credentials route fetches its token.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TokenEndpoint {
    /// The token URL.
    pub uri: String,
    /// The cluster it is fetched through.
    pub upstream: EgressUpstream,
}

/// A consumer of the secret server: the socket it asks on, in the volume only it and the server
/// mount. The mount is the caller's identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SdsConsumer {
    /// The consumer's name: the egress Envoy's compose service, or a module's id.
    pub name: String,
    /// The module, for a module consumer; `None` for the egress Envoy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub module: Option<ModuleId>,
    /// The containers that mount the socket: the module's container services. Empty for the
    /// egress Envoy, whose own fragment mounts it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub services: Vec<String>,
    /// The secrets the server serves it on this socket, and the variables holding them.
    pub secrets: Vec<SdsSecret>,
    /// The keys the server's key service wraps and unwraps under for it on this socket, and the
    /// variables holding their keyrings.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<ServedKey>,
    /// The named volume holding its socket (`sds-<consumer>`).
    pub volume: String,
    /// Where the server mounts the volume.
    pub server_dir: String,
    /// The socket's path in the server.
    pub server_socket: String,
    /// Where the consumer mounts the volume.
    pub dir: String,
    /// The socket's path in the consumer.
    pub socket: String,
}

impl SdsConsumer {
    fn new(
        name: &str,
        module: Option<ModuleId>,
        services: Vec<String>,
        secrets: Vec<SdsSecret>,
        keys: Vec<ServedKey>,
    ) -> Self {
        let server_dir = format!("{SDS_DIR}/{name}");
        SdsConsumer {
            name: name.to_string(),
            module,
            services,
            secrets,
            keys,
            volume: format!("sds-{name}"),
            server_socket: format!("{server_dir}/{SDS_SOCKET}"),
            server_dir,
            dir: SDS_DIR.to_string(),
            socket: format!("{SDS_DIR}/{SDS_SOCKET}"),
        }
    }
}

/// A key the secret server's key service holds for a consumer: its name, and the variable
/// holding its keyring (see [`keyring_var`]). Names only, never key material.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ServedKey {
    /// The key's name, which the consumer asks for, e.g. `"server-kek"`.
    pub name: String,
    /// The variable holding its keyring.
    pub env: String,
    /// The variable the consumer finds [`name`](Self::name) in (see [`key_name_var`]).
    pub name_var: String,
    /// Where its first version comes from when the environment has no keyring for it yet
    /// (see [`Provides::key_imports`](crate::Provides::key_imports)).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub import: Option<KeyImport>,
}

/// The egress-gateway module the catalog offers, if any.
pub(super) fn egress_provider(catalog: &Catalog) -> Option<ModuleId> {
    catalog
        .providers_of(Role::EGRESS)
        .first()
        .map(|m| (*m).clone())
}

/// Whether some module's services declare an outside endpoint with an injected credential,
/// which needs the egress gateway.
pub(super) fn injects_credentials(services: &BTreeMap<ModuleId, Vec<ServiceSpec>>) -> bool {
    services
        .values()
        .flatten()
        .flat_map(|s| &s.endpoints)
        .any(|e| e.route.inject_credential.is_some())
}

/// What one module asks of the secret server: its [SDS secrets](crate::Provides::sds_secrets)
/// and its [keys](crate::Provides::keys).
pub(super) struct SdsDemand {
    pub(super) module: ModuleId,
    pub(super) secrets: Vec<SdsSecret>,
    pub(super) keys: Vec<String>,
    /// The module's [key imports](crate::Provides::key_imports), checked by [`key_imports`].
    pub(super) imports: BTreeMap<String, KeyImport>,
}

/// What the settled graph's modules declare that the secret server serves: each module's
/// [SDS secrets](crate::Provides::sds_secrets) and [keys](crate::Provides::keys), and the
/// secrets it declares (to check them against).
pub(super) struct SdsDemands<'a> {
    /// Each module that reads secrets over SDS or seals under keys, in graph order.
    pub(super) sds: Vec<SdsDemand>,
    /// Each module's declared secrets.
    pub(super) declared: &'a BTreeMap<ModuleId, Vec<DeclaredSecret>>,
}

/// Plan the egress gateway: the selection's routes, then those its modules' outside endpoints
/// declare, validated against the settled graph, and the secret server's consumers. `None`
/// when `module` (the egress module) isn't in it.
pub(super) fn plan_egress(
    selection: &Selection,
    catalog: &Catalog,
    stack: &StackFacts,
    services: &BTreeMap<ModuleId, Vec<ServiceSpec>>,
    secrets: Option<&Secrets>,
    demands: &SdsDemands<'_>,
) -> Result<Option<EgressConfig>, PlanError> {
    let Some(module) = egress_provider(catalog).filter(|m| stack.has(m.as_str())) else {
        if let Some(route) = selection.egress.routes.first() {
            return Err(invalid(&route.name, "the catalog has no egress gateway"));
        }
        if let Some(demand) = demands.sds.first() {
            let reason = "the catalog has no egress gateway to run the secret server";
            return Err(match (demand.secrets.first(), demand.keys.first()) {
                (Some(secret), _) => invalid_sds(&demand.module, &secret.name, reason),
                (None, Some(key)) => invalid_key(&demand.module, key, reason),
                (None, None) => invalid_sds(&demand.module, "", reason),
            });
        }
        return Ok(None);
    };
    let mut routes: Vec<EgressRoute> = selection.egress.routes.clone();
    routes.extend(declared_routes(services));

    let mut names: BTreeMap<String, &str> = BTreeMap::new();
    let mut planned = Vec::with_capacity(routes.len());
    let mut unresolved = BTreeSet::new();
    let mut missing = BTreeSet::new();
    for (i, route) in routes.iter().enumerate() {
        check_name(&route.name)?;
        if let Some(other) = names.insert(egress_url_var(&route.name), &route.name) {
            return Err(invalid(
                &route.name,
                &format!("its name is taken by route `{other}`"),
            ));
        }
        if !route.path.starts_with('/') {
            return Err(invalid(&route.name, "its `path` must start with `/`"));
        }
        check_credential(route)?;
        for consumer in &route.consumers {
            check_consumer(&route.name, consumer, &module, stack, services)?;
            if let Some(m) = catalog.get(consumer) {
                m.check_egress_route(route)
                    .map_err(|reason| invalid(&route.name, &reason))?;
            }
        }
        let (function, origin) = match (&route.function, &route.upstream) {
            (Some(slug), None) => {
                let origin = function_origin(&route.name, slug, catalog, stack)?;
                (Some(slug.clone()), origin)
            }
            (None, Some(upstream)) => (None, upstream.clone()),
            _ => {
                return Err(invalid(
                    &route.name,
                    "set exactly one of `function` and `upstream`",
                ));
            }
        };
        let origin = match secrets {
            Some(secrets) => substitute(&origin, secrets, &mut missing),
            None => origin,
        };
        let cluster = format!("route_{}", cluster_suffix(&route.name));
        let upstream = upstream_of(&route.name, &origin, cluster.clone(), &mut unresolved)?;
        let token_endpoint = match &route.credential {
            InjectedCredential::OAuth2ClientCredentials { token_url, .. } => Some(TokenEndpoint {
                uri: token_url.clone(),
                upstream: token_upstream(&route.name, token_url, format!("{cluster}_token"))?,
            }),
            _ => None,
        };
        // Matched segment-wise: `/v1` forwards `/v1` and `/v1/…`, never `/v1beta`.
        let path = match route.path.trim_end_matches('/') {
            "" => "/".to_string(),
            trimmed => trimmed.to_string(),
        };
        let port = u16::try_from(i)
            .ok()
            .and_then(|i| EGRESS_FIRST_PORT.checked_add(i))
            .ok_or_else(|| invalid(&route.name, "too many egress routes"))?;
        planned.push(PlannedEgressRoute {
            name: route.name.clone(),
            port,
            url: format!("http://{EGRESS_SERVICE}:{port}{path}"),
            path,
            url_var: egress_url_var(&route.name),
            function,
            origin,
            upstream,
            credential: route.credential.clone(),
            token_endpoint,
            consumers: route.consumers.clone(),
            mcp: route.mcp.clone(),
        });
    }
    if !missing.is_empty() {
        return Err(PlanError::UnresolvedSecrets {
            vars: missing.into_iter().collect(),
        });
    }
    let envoy = SdsConsumer::new(
        EGRESS_SERVICE,
        None,
        Vec::new(),
        planned
            .iter()
            .map(|r| SdsSecret::new(&r.name, r.credential.secret_var()))
            .collect(),
        Vec::new(),
    );
    let mut sds_consumers = vec![envoy];
    for demand in &demands.sds {
        let containers = check_sds_consumer(demand, &module, stack, services, demands)?;
        let keys = check_keys(demand, &sds_consumers)?;
        sds_consumers.push(SdsConsumer::new(
            demand.module.as_str(),
            Some(demand.module.clone()),
            containers,
            demand.secrets.clone(),
            keys,
        ));
    }
    let mut secrets: Vec<String> = Vec::new();
    for var in sds_consumers.iter().flat_map(|c| {
        c.secrets
            .iter()
            .map(|s| &s.env)
            .chain(c.keys.iter().map(|k| &k.env))
    }) {
        if !secrets.contains(var) {
            secrets.push(var.clone());
        }
    }
    let clusters = planned
        .iter()
        .flat_map(|r| {
            std::iter::once(r.upstream.clone())
                .chain(r.token_endpoint.iter().map(|t| t.upstream.clone()))
        })
        .collect();
    let host = selection.egress.host_source_port().map(|port| HostSource {
        port,
        url: format!("http://{HOST_SOURCE_HOST}:{port}"),
        token_env: HOST_SOURCE_TOKEN_VAR.to_string(),
    });
    Ok(Some(EgressConfig {
        module,
        clusters,
        secrets,
        host,
        routes: planned,
        sds_consumers,
        admin_port: EGRESS_ADMIN_PORT,
        unresolved_upstreams: unresolved.into_iter().collect(),
    }))
}

/// The routes modules declare: one per outside endpoint with an injected credential, named
/// `<module>-<endpoint>`, for its module alone, forwarding the service's base path.
fn declared_routes(services: &BTreeMap<ModuleId, Vec<ServiceSpec>>) -> Vec<EgressRoute> {
    let mut out = Vec::new();
    for (module, specs) in services {
        for spec in specs {
            let Placement::External { origin } = &spec.placement else {
                continue;
            };
            for endpoint in &spec.endpoints {
                let Some(credential) = &endpoint.route.inject_credential else {
                    continue;
                };
                out.push(EgressRoute {
                    name: format!("{module}-{}", endpoint.id),
                    function: None,
                    upstream: Some(origin.clone()),
                    path: if spec.base_path.is_empty() {
                        "/".into()
                    } else {
                        spec.base_path.clone()
                    },
                    consumers: vec![module.clone()],
                    credential: credential.clone(),
                    mcp: None,
                });
            }
        }
    }
    out
}

fn invalid(route: &str, reason: &str) -> PlanError {
    PlanError::InvalidEgressRoute {
        route: route.to_string(),
        reason: reason.to_string(),
    }
}

fn invalid_key(module: &ModuleId, key: &str, reason: &str) -> PlanError {
    PlanError::InvalidModuleKey {
        module: module.clone(),
        key: key.to_string(),
        reason: reason.to_string(),
    }
}

/// A module's keys: each named once, by a valid name of at most [`MAX_KEY_NAME_LEN`] bytes,
/// whose variables ([`key_name_var`], [`keyring_var`]) no other key of the plan takes.
fn check_keys(demand: &SdsDemand, planned: &[SdsConsumer]) -> Result<Vec<ServedKey>, PlanError> {
    let module = &demand.module;
    let mut out: Vec<ServedKey> = Vec::with_capacity(demand.keys.len());
    for key in &demand.keys {
        if !is_name(key) {
            return Err(invalid_key(
                module,
                key,
                "a key's name is letters, digits, `-` and `_`, starting with a letter",
            ));
        }
        if key.len() > MAX_KEY_NAME_LEN {
            return Err(invalid_key(
                module,
                key,
                &format!("a key's name is at most {MAX_KEY_NAME_LEN} bytes"),
            ));
        }
        let served = ServedKey {
            name: key.clone(),
            env: keyring_var(module, key),
            name_var: key_name_var(key),
            import: demand.imports.get(key).cloned(),
        };
        if let Some(other) = out.iter().find(|k| k.name_var == served.name_var) {
            return Err(invalid_key(
                module,
                key,
                &if other.name == *key {
                    "it is named twice".to_string()
                } else {
                    format!(
                        "it and key `{}` share the variable `{}`",
                        other.name, served.name_var
                    )
                },
            ));
        }
        if let Some((owner, other)) = planned.iter().find_map(|c| {
            c.keys
                .iter()
                .find(|k| k.env == served.env)
                .map(|k| (c.name.as_str(), k.name.as_str()))
        }) {
            return Err(invalid_key(
                module,
                key,
                &format!(
                    "its keyring variable `{}` is taken by key `{other}` of `{owner}`",
                    served.env
                ),
            ));
        }
        out.push(served);
    }
    Ok(out)
}

/// A module's key imports: each of a key it declares, from a variable by a valid name, under an
/// alias a keyring accepts (1 to 255 bytes, not of the form `<key>@v<n>`, which versions take).
pub(super) fn key_imports(
    module: &ModuleId,
    declared: &[String],
    imports: &BTreeMap<String, KeyImport>,
) -> Result<BTreeMap<String, KeyImport>, PlanError> {
    for (key, import) in imports {
        if !declared.contains(key) {
            return Err(invalid_key(
                module,
                key,
                "it has an import but isn't one of the module's keys",
            ));
        }
        if !is_var_name(&import.from) {
            return Err(invalid_key(
                module,
                key,
                &format!(
                    "its import's variable `{}` is not a variable name",
                    import.from
                ),
            ));
        }
        if let Some(alias) = &import.alias {
            let versioned = alias
                .rsplit_once("@v")
                .is_some_and(|(_, n)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
            if alias.is_empty() || alias.len() > 255 || versioned {
                return Err(invalid_key(
                    module,
                    key,
                    &format!(
                        "its import's alias `{alias}` must be 1 to 255 bytes and not of the \
                         form `<key>@v<n>`"
                    ),
                ));
            }
        }
    }
    Ok(imports.clone())
}

/// Letters, digits and `_`, not starting with a digit.
fn is_var_name(name: &str) -> bool {
    name.bytes()
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn invalid_sds(module: &ModuleId, secret: &str, reason: &str) -> PlanError {
    PlanError::InvalidSdsSecret {
        module: module.clone(),
        secret: secret.to_string(),
        reason: reason.to_string(),
    }
}

/// Letters, digits, `-` and `_`, starting with a letter.
fn is_name(name: &str) -> bool {
    name.bytes().next().is_some_and(|b| b.is_ascii_alphabetic())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// A module reading secrets over SDS must run containers in the environment (they mount its
/// socket), must not be the egress module, and must name each secret once, by a valid name,
/// held by a variable it declares. Returns its container services.
fn check_sds_consumer(
    demand: &SdsDemand,
    egress: &ModuleId,
    stack: &StackFacts,
    services: &BTreeMap<ModuleId, Vec<ServiceSpec>>,
    demands: &SdsDemands<'_>,
) -> Result<Vec<String>, PlanError> {
    let (consumer, wanted) = (&demand.module, demand.secrets.as_slice());
    // A refusal names the first secret, or the first key of a module asking for keys alone.
    let refuse = |reason: &str| match (wanted.first(), demand.keys.first()) {
        (None, Some(key)) => invalid_key(consumer, key, reason),
        (first, _) => invalid_sds(consumer, first.map_or("", |s| s.name.as_str()), reason),
    };
    if consumer == egress || consumer.as_str() == EGRESS_SERVICE {
        return Err(refuse(
            "the egress gateway can't consume the secret server over a module socket",
        ));
    }
    let containers: Vec<String> = services
        .get(consumer)
        .into_iter()
        .flatten()
        .filter_map(|s| match &s.placement {
            Placement::Container { service } => Some(service.clone()),
            _ => None,
        })
        .collect();
    if containers.is_empty() || !stack.runs(consumer.as_str()) {
        return Err(refuse(
            "the module runs no container in the environment to mount the socket in",
        ));
    }
    let declared = demands
        .declared
        .get(consumer)
        .map_or(&[][..], Vec::as_slice);
    let mut seen = BTreeSet::new();
    for secret in wanted {
        if !is_name(&secret.name) {
            return Err(invalid_sds(
                consumer,
                &secret.name,
                "an SDS secret's name is letters, digits, `-` and `_`, starting with a letter",
            ));
        }
        if !seen.insert(secret.name.as_str()) {
            return Err(invalid_sds(consumer, &secret.name, "it is named twice"));
        }
        if !declared.iter().any(|d| d.name == secret.env) {
            return Err(invalid_sds(
                consumer,
                &secret.name,
                &format!(
                    "its variable `{}` is not one of the module's declared secrets",
                    secret.env
                ),
            ));
        }
    }
    Ok(containers)
}

/// A route's name names its listener, cluster, SDS secret and URL variable: letters, digits,
/// `-` and `_`, starting with a letter.
fn check_name(name: &str) -> Result<(), PlanError> {
    if is_name(name) {
        Ok(())
    } else {
        Err(invalid(
            name,
            "a route name is letters, digits, `-` and `_`, starting with a letter",
        ))
    }
}

/// A credential's secret names a variable (it is never a value), and an OAuth2 token endpoint
/// is a URL.
fn check_credential(route: &EgressRoute) -> Result<(), PlanError> {
    let var = route.credential.secret_var();
    let is_var = var
        .bytes()
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && var.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    if !is_var {
        return Err(invalid(
            &route.name,
            &format!("its credential's secret `{var}` must name a variable, never hold a value"),
        ));
    }
    if let InjectedCredential::Header { name, .. } = &route.credential
        && (name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
    {
        return Err(invalid(
            &route.name,
            &format!("`{name}` is not a header name"),
        ));
    }
    Ok(())
}

/// A consumer must be in the environment and run a container there to call from.
fn check_consumer(
    route: &str,
    consumer: &ModuleId,
    egress: &ModuleId,
    stack: &StackFacts,
    services: &BTreeMap<ModuleId, Vec<ServiceSpec>>,
) -> Result<(), PlanError> {
    let unknown = |reason: &str| PlanError::UnknownEgressConsumer {
        route: route.to_string(),
        module: consumer.clone(),
        reason: reason.to_string(),
    };
    if !stack.has(consumer.as_str()) {
        return Err(unknown("is not in the environment"));
    }
    if consumer == egress {
        return Err(unknown("is the egress gateway itself"));
    }
    let runs = services.get(consumer).is_some_and(|specs| {
        specs
            .iter()
            .any(|s| matches!(s.placement, Placement::Container { .. }))
    });
    if !runs || !stack.runs(consumer.as_str()) {
        return Err(unknown("runs nothing in the environment to call it from"));
    }
    Ok(())
}

/// The origin of the function `slug`: a function-hosted module's URL when one runs as it, else
/// `${<SLUG>_FUNCTION_URL}`. A malformed slug, or one a module in the environment runs as a
/// container, is a [`PlanError::UnknownEgressFunction`].
fn function_origin(
    route: &str,
    slug: &str,
    catalog: &Catalog,
    stack: &StackFacts,
) -> Result<String, PlanError> {
    let unknown = |reason: String| PlanError::UnknownEgressFunction {
        route: route.to_string(),
        function: slug.to_string(),
        reason,
    };
    let bytes = slug.as_bytes();
    let well_formed = !bytes.is_empty()
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
        && bytes[0] != b'-'
        && bytes[bytes.len() - 1] != b'-';
    if !well_formed {
        return Err(unknown(
            "is not a function slug (lowercase letters, digits and `-`)".into(),
        ));
    }
    if let Some(hosted) = stack.functions.values().find(|f| f.slug == slug) {
        return Ok(hosted.url.clone());
    }
    if let Some(module) = catalog
        .modules()
        .iter()
        .find(|m| m.provides().function.as_deref() == Some(slug) && stack.runs(m.id().as_str()))
    {
        return Err(unknown(format!(
            "is `{}`'s, which runs in this environment as a container, not as a function",
            module.id()
        )));
    }
    Ok(format!("${{{}}}", function_url_var(slug)))
}

/// `s` with every `${VAR}` `secrets` supplies substituted; each one it doesn't goes in
/// `missing`.
fn substitute(s: &str, secrets: &Secrets, missing: &mut BTreeSet<String>) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = 0;
    for (span, name) in env_refs(s) {
        out.push_str(&s[rest..span.start]);
        match secrets.get(name) {
            Some(value) => out.push_str(value),
            None => {
                missing.insert(name.to_string());
                out.push_str(&s[span.clone()]);
            }
        }
        rest = span.end;
    }
    out.push_str(&s[rest..]);
    out
}

/// The cluster reaching a route's `origin`: its host and port, over TLS (verified, with SNI)
/// for `https`. A whole `${VAR}` stands for an `https` origin not resolved yet (recorded in
/// `unresolved`). Anything else that isn't an origin at its root is an
/// [`PlanError::InvalidEgressRoute`].
fn upstream_of(
    route: &str,
    origin: &str,
    cluster: String,
    unresolved: &mut BTreeSet<String>,
) -> Result<EgressUpstream, PlanError> {
    if let Some(var) = crate::address::whole_ref(origin) {
        unresolved.insert(var.to_string());
        return Ok(EgressUpstream {
            cluster,
            host: origin.to_string(),
            port: 443,
            tls: Some(origin.to_string()),
        });
    }
    let (host, port, tls) = super::gateway::external_upstream(origin).ok_or_else(|| {
        invalid(
            route,
            &format!("`{origin}` is not an `http(s)://host[:port]` origin"),
        )
    })?;
    Ok(EgressUpstream {
        cluster,
        host,
        port,
        tls,
    })
}

/// The cluster an OAuth2 token endpoint is fetched through.
fn token_upstream(
    route: &str,
    token_url: &str,
    cluster: String,
) -> Result<EgressUpstream, PlanError> {
    let bad = || {
        invalid(
            route,
            &format!("`{token_url}` is not an `http(s)` token URL"),
        )
    };
    let url = url::Url::parse(token_url).map_err(|_| bad())?;
    let host = url.host_str().ok_or_else(bad)?.to_string();
    let port = url.port_or_known_default().ok_or_else(bad)?;
    let tls = match url.scheme() {
        "https" => Some(host.clone()),
        "http" => None,
        _ => return Err(bad()),
    };
    Ok(EgressUpstream {
        cluster,
        host,
        port,
        tls,
    })
}

/// A route name as part of a cluster name (`-` as `_`).
fn cluster_suffix(name: &str) -> String {
    name.replace('-', "_")
}
