//! The single source of addressing truth: [`address`] and [`address_direct`].
//!
//! The platform's posture is *everything goes through the gateway* — so the
//! default, [`address`], routes through the gateway whenever the coordinator's
//! [`RoutePlan`] assigns the endpoint a route, and only falls back to a direct
//! address when it doesn't (a database port, an in-process service, anything the
//! planner left off the surface). [`address_direct`] is the explicit escape hatch
//! for the rare case a gatewayed endpoint must be reached straight.
//!
//! A **direct** address depends on the caller's [`Vantage`] and the callee's
//! [`Placement`] — the rules both sibling tools otherwise hand-author at each call
//! site:
//!
//! | from \ to        | `Container`                 | `Host` / `InProcess`          |
//! |------------------|-----------------------------|-------------------------------|
//! | `Container`      | `<service_dns>:<internal>`  | `host.docker.internal:<host>` |
//! | `Host`           | `localhost:<host>`          | `127.0.0.1:<host‖internal>`   |
//! | `InProcess`      | `localhost:<host>`          | `127.0.0.1:<host‖internal>`   |
//!
//! A **gateway** address ignores the callee's own placement — it is *the gateway's*
//! address (for the route's [`Listener`]), plus the route prefix: the gateway is
//! reached at `envoy:10000` from a container and at `localhost:<gateway_host_port>`
//! from the host / in-process (or the dedicated listener's port for a
//! [`Listener::Dedicated`] route).
//!
//! The functions are pure. Runtime facts — the gateway's host-published port, a
//! dynamically-allocated host port — arrive via [`TopologyCtx`]; this crate never
//! discovers them.

use url::Url;

use crate::model::endpoint::{Endpoint, Scheme};
use crate::model::placement::{Placement, Vantage};
use crate::model::role::ServiceSpec;
use crate::plan::routing::{AssignedRoute, Listener, RoutePlan};

/// The conventional DNS hostname a container uses to reach the host machine.
const HOST_GATEWAY_DNS: &str = "host.docker.internal";

/// Resolved, environment-wide facts the resolver needs but cannot derive from the
/// model alone. The consuming tool fills this in once per environment.
#[derive(Clone, Debug)]
pub struct TopologyCtx {
    /// The gateway's compose service / DNS name, as reached from inside the
    /// compose network (e.g. `"envoy"`).
    pub gateway_service: String,
    /// The gateway's listening port inside the compose network (e.g. `10000`).
    pub gateway_internal_port: u16,
    /// The gateway's host-published port — what a host-side or in-process caller
    /// reaches it at (e.g. `9080`).
    pub gateway_host_port: u16,
}

/// What can go wrong resolving an address.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AddressError {
    /// The named endpoint is not on the callee service.
    #[error("service `{service}` has no endpoint `{endpoint}`")]
    UnknownEndpoint { service: String, endpoint: String },
    /// A host-vantage caller needs the callee's host-published port, but the
    /// endpoint declares none.
    #[error(
        "endpoint `{endpoint}` on `{service}` has no host_port, required to reach it from the host"
    )]
    NoHostPort { service: String, endpoint: String },
    /// The resolved components did not form a valid URL.
    #[error("could not build a URL from the resolved address `{0}`")]
    InvalidUrl(String),
}

/// Resolve the URL a caller at `from` should use to reach `endpoint_id` on `to`,
/// **through the gateway when the coordinator assigned it a route**.
///
/// This is the default: when `plan` has an [`AssignedRoute`] for this endpoint, the
/// address is the gateway listener's plus the assigned prefix; otherwise it
/// resolves directly (e.g. a database port, an in-process service, or anything the
/// planner did not front). To force a direct address for a gatewayed endpoint — the
/// rare exception — use [`address_direct`].
pub fn address(
    from: Vantage,
    to: &ServiceSpec,
    endpoint_id: &str,
    plan: &RoutePlan,
    ctx: &TopologyCtx,
) -> Result<Url, AddressError> {
    let endpoint = lookup(to, endpoint_id)?;
    let target = match plan.get(&to.name, endpoint_id) {
        Some(route) => gateway_target(from, route, endpoint, ctx),
        None => direct_target(from, to, endpoint)?,
    };
    finish(endpoint.scheme, target)
}

/// Resolve the **direct** URL a caller at `from` should use to reach `endpoint_id`
/// on `to`, bypassing the gateway even if the endpoint is gatewayed.
///
/// The explicit escape hatch from the gateway-by-default posture of [`address`].
pub fn address_direct(
    from: Vantage,
    to: &ServiceSpec,
    endpoint_id: &str,
) -> Result<Url, AddressError> {
    let endpoint = lookup(to, endpoint_id)?;
    let target = direct_target(from, to, endpoint)?;
    finish(endpoint.scheme, target)
}

/// A resolved (host, port, path) triple before it is rendered to a [`Url`].
type Target = (String, u16, String);

fn lookup<'a>(to: &'a ServiceSpec, endpoint_id: &str) -> Result<&'a Endpoint, AddressError> {
    to.endpoint(endpoint_id)
        .ok_or_else(|| AddressError::UnknownEndpoint {
            service: to.name.clone(),
            endpoint: endpoint_id.to_string(),
        })
}

fn finish(scheme: Scheme, target: Target) -> Result<Url, AddressError> {
    let (host, port, path) = target;
    build_url(scheme, &host, port, &path)
        .ok_or_else(|| AddressError::InvalidUrl(format!("{host}:{port}{path}")))
}

/// The (host, port, path) for a direct reach — straight to the callee.
fn direct_target(
    from: Vantage,
    to: &ServiceSpec,
    endpoint: &Endpoint,
) -> Result<Target, AddressError> {
    let host = match (from, &to.placement) {
        // Inside the compose network, a container is reached by its DNS name on
        // its internal port.
        (Vantage::Container, Placement::Container { service }) => service.clone(),
        // Crossing the boundary toward the host (a host process or the in-process
        // host itself) — the container reaches it via the host-gateway DNS name on
        // the host-published port.
        (Vantage::Container, Placement::Host | Placement::InProcess) => {
            HOST_GATEWAY_DNS.to_string()
        }
        // A host-side or in-process caller reaching a container goes to the
        // container's host-published port on localhost.
        (Vantage::Host | Vantage::InProcess, Placement::Container { .. }) => {
            "localhost".to_string()
        }
        // Host/in-process caller reaching a host process or another in-process
        // service: loopback.
        (Vantage::Host | Vantage::InProcess, Placement::Host | Placement::InProcess) => {
            "127.0.0.1".to_string()
        }
    };

    // Port: a container-internal target uses its internal port; everything that
    // crosses to or lives on the host uses the host-published port.
    let port = match (from, &to.placement) {
        (Vantage::Container, Placement::Container { .. }) => endpoint.internal_port,
        // A host/in-process service's bound port is its host port; fall back to
        // the internal port when none is declared separately (they coincide for a
        // host-bound sidecar).
        (_, Placement::Host | Placement::InProcess) => {
            endpoint.host_port.unwrap_or(endpoint.internal_port)
        }
        // Reaching a container from the host requires its host-published port.
        (Vantage::Host | Vantage::InProcess, Placement::Container { .. }) => {
            endpoint.host_port.ok_or_else(|| AddressError::NoHostPort {
                service: to.name.clone(),
                endpoint: endpoint.id.clone(),
            })?
        }
    };

    Ok((host, port, endpoint.path.clone()))
}

/// The (host, port, path) for a gateway reach — the gateway listener's address
/// plus the coordinator-assigned prefix. The callee's own placement is irrelevant
/// here; what matters is the assigned route's [`Listener`].
fn gateway_target(
    from: Vantage,
    route: &AssignedRoute,
    endpoint: &Endpoint,
    ctx: &TopologyCtx,
) -> Target {
    let (host, port) = match (from, &route.listener) {
        // From inside the compose network, the gateway is just another container.
        // The shared listener is on its internal port; a dedicated listener listens
        // on its declared port (we control both ends, so no remap).
        (Vantage::Container, Listener::Shared) => {
            (ctx.gateway_service.clone(), ctx.gateway_internal_port)
        }
        (Vantage::Container, Listener::Dedicated { port }) => (ctx.gateway_service.clone(), *port),
        // From the host or in-process, the gateway is reached on localhost: the
        // shared listener at its host-published port, a dedicated one at its port.
        (Vantage::Host | Vantage::InProcess, Listener::Shared) => {
            ("localhost".to_string(), ctx.gateway_host_port)
        }
        (Vantage::Host | Vantage::InProcess, Listener::Dedicated { port }) => {
            ("localhost".to_string(), *port)
        }
    };

    // Client-facing path is the route prefix followed by the endpoint's own path.
    // For a UI the prefix is the service's base path and the endpoint path is
    // typically empty; for an API the endpoint path nests below the prefix. The
    // gateway applies any upstream `rewrite` itself — that is not part of the
    // client-facing address. Collapse a double slash at the join (a prefix ending
    // in `/` meeting a path beginning with `/`) — the `url` crate does not
    // normalize interior `//`, so we do it here to keep the address well-formed.
    let path = join_path(&route.prefix, &endpoint.path);
    (host, port, path)
}

/// Join a route prefix and an endpoint path, collapsing a single `/` at the seam
/// so a trailing-slash prefix and a leading-slash path don't yield `//`.
fn join_path(prefix: &str, path: &str) -> String {
    match (prefix.ends_with('/'), path.starts_with('/')) {
        (true, true) => format!("{}{}", prefix, &path[1..]),
        _ => format!("{prefix}{path}"),
    }
}

/// Assemble a URL from parts.
///
/// `tcp://` targets carry no path; a non-empty path on a `Tcp` endpoint is a
/// modeling error (a raw port has no path), so we debug-assert rather than
/// silently drop it.
///
/// Note: the `url` crate elides a scheme's *default* port from the rendered
/// string (`http://h:80` → `http://h`). That is correct URL semantics, and these
/// services never bind 80/443 in practice; callers that need a bare `host:port`
/// authority should read [`Url::host`]/[`Url::port_or_known_default`] rather than
/// parse the string.
fn build_url(scheme: Scheme, host: &str, port: u16, path: &str) -> Option<Url> {
    debug_assert!(
        scheme != Scheme::Tcp || path.is_empty(),
        "tcp endpoint carries a non-empty path `{path}`; tcp targets have no path"
    );
    let base = format!("{}://{host}:{port}", scheme.url_scheme());
    let mut url = Url::parse(&base).ok()?;
    if scheme != Scheme::Tcp && !path.is_empty() {
        url.set_path(path);
    }
    Some(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::endpoint::{RouteIntent, Scheme};
    use crate::model::placement::Placement;
    use crate::model::role::Role;
    use crate::plan::routing::{AssignedRoute, Listener, RoutePlan};

    fn ctx() -> TopologyCtx {
        TopologyCtx {
            gateway_service: "envoy".into(),
            gateway_internal_port: 10000,
            gateway_host_port: 9080,
        }
    }

    /// A shared-listener API route the coordinator assigned at `prefix`.
    fn api_route(prefix: &str) -> AssignedRoute {
        AssignedRoute {
            prefix: prefix.into(),
            rewrite: None,
            listener: Listener::Shared,
            base_path: None,
        }
    }

    /// A host-bound catalog sidecar (hydrofoil's Unity Catalog): bound to a
    /// dynamic host port, reached over REST. Reached directly by the desktop, so
    /// its endpoint is [`RouteIntent::Internal`]. `host_port == internal_port`
    /// because it is a single host process.
    fn host_catalog(port: u16) -> ServiceSpec {
        ServiceSpec {
            name: "unity-catalog".into(),
            role: Role::new("data_catalog"),
            placement: Placement::Host,
            endpoints: vec![Endpoint {
                id: "rest".into(),
                scheme: Scheme::Http,
                internal_port: port,
                host_port: Some(port),
                intent: RouteIntent::Internal,
                path: "/api/2.1/unity-catalog/".into(),
                mount_prefix: None,
                rewrite: crate::model::endpoint::Rewrite::Inherit,
            }],
            depends_on: vec![],
            base_path: String::new(),
        }
    }

    /// Postgres in a container: reached only inside the network on `db:5432`.
    fn container_postgres() -> ServiceSpec {
        ServiceSpec {
            name: "postgres".into(),
            role: Role::new("relational_db"),
            placement: Placement::Container {
                service: "db".into(),
            },
            endpoints: vec![Endpoint::internal("sql", Scheme::Tcp, 5432, None)],
            depends_on: vec![],
            base_path: String::new(),
        }
    }

    /// Marquez in a container; its lineage endpoint is an API the gateway can front.
    fn container_marquez() -> ServiceSpec {
        ServiceSpec {
            name: "marquez".into(),
            role: Role::new("lineage"),
            placement: Placement::Container {
                service: "marquez-api".into(),
            },
            endpoints: vec![Endpoint::api(
                "lineage",
                5000,
                "/api/2.1/lineage",
                crate::model::endpoint::Rewrite::Inherit,
            )],
            depends_on: vec![],
            base_path: String::new(),
        }
    }

    /// MLflow in a container; its UI can take a base path (`--static-prefix`), so
    /// the planner can front it on the shared gateway without breaking links.
    fn container_mlflow_ui() -> ServiceSpec {
        ServiceSpec {
            name: "mlflow".into(),
            role: Role::new("experiment_tracking"),
            placement: Placement::Container {
                service: "mlflow".into(),
            },
            endpoints: vec![Endpoint::ui_prefixable("ui", 5000, None)],
            depends_on: vec![],
            base_path: String::new(),
        }
    }

    /// `address` (gateway-when-the-plan-assigns-it) as a string.
    fn addr(from: Vantage, to: &ServiceSpec, ep: &str, plan: &RoutePlan) -> String {
        address(from, to, ep, plan, &ctx()).unwrap().to_string()
    }

    /// `address_direct` as a string.
    fn direct(from: Vantage, to: &ServiceSpec, ep: &str) -> String {
        address_direct(from, to, ep).unwrap().to_string()
    }

    // --- Direct addresses: the cross-vantage matrix, asserting the exact strings
    // the sibling tools use today. With an empty plan nothing is gatewayed, so the
    // default `address` resolves direct. ---

    #[test]
    fn container_to_host_catalog_uses_host_docker_internal() {
        // hydrofoil's UC_HOST_URL: mlflow/marquez (containers) → UC (host).
        let uc = host_catalog(54321);
        assert_eq!(
            addr(Vantage::Container, &uc, "rest", &RoutePlan::new()),
            "http://host.docker.internal:54321/api/2.1/unity-catalog/"
        );
    }

    #[test]
    fn in_process_to_host_catalog_uses_loopback() {
        // hydrofoil's HostConfig.unity_endpoint: the embedded engine → UC sidecar.
        let uc = host_catalog(54321);
        assert_eq!(
            addr(Vantage::InProcess, &uc, "rest", &RoutePlan::new()),
            "http://127.0.0.1:54321/api/2.1/unity-catalog/"
        );
    }

    #[test]
    fn host_to_host_catalog_uses_loopback() {
        // hydrofoil's proxy_request: the UI (host) → UC sidecar.
        let uc = host_catalog(54321);
        assert_eq!(
            addr(Vantage::Host, &uc, "rest", &RoutePlan::new()),
            "http://127.0.0.1:54321/api/2.1/unity-catalog/"
        );
    }

    #[test]
    fn container_to_container_uses_compose_dns() {
        // mlflow/marquez (container) → postgres (container) on db:5432.
        let pg = container_postgres();
        assert_eq!(
            addr(Vantage::Container, &pg, "sql", &RoutePlan::new()),
            "tcp://db:5432"
        );
    }

    #[test]
    fn host_to_container_without_host_port_errors() {
        // Postgres is not host-published; a host caller cannot reach it directly.
        let pg = container_postgres();
        let err = address(Vantage::Host, &pg, "sql", &RoutePlan::new(), &ctx()).unwrap_err();
        assert!(matches!(err, AddressError::NoHostPort { .. }));
    }

    // --- Gateway-by-default: when the coordinator's plan assigns a route, the
    // endpoint resolves through the gateway with no caller opt-in. ---

    #[test]
    fn in_process_to_marquez_defaults_through_gateway() {
        // hydrofoil's lineage sink: the embedded engine emits to Marquez through
        // the Envoy gateway on the host-published port.
        let marquez = container_marquez();
        let mut plan = RoutePlan::new();
        plan.assign("marquez", "lineage", api_route("/api/v1/lineage"));
        assert_eq!(
            addr(Vantage::InProcess, &marquez, "lineage", &plan),
            "http://localhost:9080/api/v1/lineage"
        );
    }

    #[test]
    fn container_to_gatewayed_service_uses_envoy_internal() {
        // The address trestle's in-container app/notebooks NEED — envoy:10000,
        // not localhost:9080. The bug-class fix, now the DEFAULT behavior.
        let marquez = container_marquez();
        let mut plan = RoutePlan::new();
        plan.assign("marquez", "lineage", api_route("/api/v1/lineage"));
        assert_eq!(
            addr(Vantage::Container, &marquez, "lineage", &plan),
            "http://envoy:10000/api/v1/lineage"
        );
    }

    // --- UI routes: the planner chose the base path; the UI serves itself under
    // it so links resolve. ---

    #[test]
    fn ui_route_fronts_at_assigned_base_path() {
        let mlflow = container_mlflow_ui();
        let mut plan = RoutePlan::new();
        plan.assign(
            "mlflow",
            "ui",
            AssignedRoute {
                prefix: "/mlflow".into(),
                rewrite: None,
                listener: Listener::Shared,
                base_path: Some("/mlflow".into()),
            },
        );
        // Host UI reaches the MLflow UI at the shared gateway under /mlflow.
        assert_eq!(
            addr(Vantage::Host, &mlflow, "ui", &plan),
            "http://localhost:9080/mlflow"
        );
        // In-container caller reaches the same UI via envoy:10000/mlflow.
        assert_eq!(
            addr(Vantage::Container, &mlflow, "ui", &plan),
            "http://envoy:10000/mlflow"
        );
    }

    #[test]
    fn dedicated_listener_uses_its_own_port() {
        // A UI the planner put on its own external port (e.g. it couldn't take a
        // base path) is reached at that port, not the shared one.
        let mlflow = container_mlflow_ui();
        let mut plan = RoutePlan::new();
        plan.assign(
            "mlflow",
            "ui",
            AssignedRoute {
                prefix: "/".into(),
                rewrite: None,
                listener: Listener::Dedicated { port: 9443 },
                base_path: Some("/".into()),
            },
        );
        // Host caller hits the dedicated host port.
        assert_eq!(
            addr(Vantage::Host, &mlflow, "ui", &plan),
            "http://localhost:9443/"
        );
        // In-container caller hits the same listener on the gateway container.
        assert_eq!(
            addr(Vantage::Container, &mlflow, "ui", &plan),
            "http://envoy:9443/"
        );
    }

    // --- Escape hatch: address_direct bypasses the gateway even for a planned
    // endpoint. ---

    #[test]
    fn address_direct_bypasses_gateway() {
        // Marquez may be gatewayed, but address_direct reaches its container
        // directly regardless of the plan.
        let marquez = container_marquez();
        assert_eq!(
            direct(Vantage::Container, &marquez, "lineage"),
            "http://marquez-api:5000/"
        );
    }

    #[test]
    fn unknown_endpoint_errors() {
        let uc = host_catalog(54321);
        let err = address(Vantage::Host, &uc, "nope", &RoutePlan::new(), &ctx()).unwrap_err();
        assert!(matches!(err, AddressError::UnknownEndpoint { .. }));
        let err = address_direct(Vantage::Host, &uc, "nope").unwrap_err();
        assert!(matches!(err, AddressError::UnknownEndpoint { .. }));
    }

    #[test]
    fn api_gateway_route_with_path_concatenates() {
        // An API endpoint fronted by the gateway that also has its own nested path.
        let mut uc = host_catalog(54321);
        uc.placement = Placement::Container {
            service: "unitycatalog".into(),
        };
        uc.endpoints[0].intent = RouteIntent::Api;
        let mut plan = RoutePlan::new();
        plan.assign("unity-catalog", "rest", api_route("/api/2.1/unity-catalog"));
        // prefix (no trailing slash) + path (leading slash) join cleanly.
        assert_eq!(
            addr(Vantage::Container, &uc, "rest", &plan),
            "http://envoy:10000/api/2.1/unity-catalog/api/2.1/unity-catalog/"
        );
    }

    #[test]
    fn gateway_join_collapses_double_slash_at_seam() {
        // A trailing-slash prefix meeting a leading-slash path must not yield
        // `//` — the url crate does not normalize interior double slashes.
        let mut svc = container_marquez();
        svc.endpoints[0].path = "/lineage".into();
        let mut plan = RoutePlan::new();
        plan.assign("marquez", "lineage", api_route("/api/v1/"));
        assert_eq!(
            addr(Vantage::Container, &svc, "lineage", &plan),
            "http://envoy:10000/api/v1/lineage"
        );
    }
}
