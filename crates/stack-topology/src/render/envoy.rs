//! The Envoy gateway bootstrap renderer, shared by every deploy target.
//!
//! Envoy is the gateway *implementation*, not a deploy target: whichever target runs the
//! stack, the gateway reads the same bootstrap, rendered here from the planner's structured
//! [`GatewayConfig`]. Pure and string-only — no I/O.

use serde::Serialize;

use crate::plan::GatewayConfig;

/// The on-disk Envoy bootstrap template, embedded at compile time. Lives at
/// `templates/gateway/bootstrap.yaml.jinja` (a sibling of `src/`), alongside the envoy
/// module's compose fragment (`templates/gateway/compose.yaml.jinja`), so the gateway's two
/// template faces sit together and are reviewable as plain files rather than inline strings.
const ENVOY_TEMPLATE: &str = include_str!("../../templates/gateway/bootstrap.yaml.jinja");

/// The flat, owned render context the Envoy template iterates. The ordering decisions
/// (longest-prefix-first routes, app cluster first) stay in Rust; the template only
/// renders what it is handed. The core plan types ([`GatewayConfig`] et al.) deliberately
/// do not derive `Serialize`, so this is the serialization boundary.
#[derive(Serialize)]
struct EnvoyCtx<'a> {
    /// Every listener to emit: the shared, path-multiplexed one first, then any dedicated
    /// listeners (one per fixed-path UI / gatewayed backend) on their own ports.
    listeners: Vec<ListenerCtx<'a>>,
    clusters: Vec<ClusterCtx<'a>>,
    /// The Envoy admin port (bound on both sides).
    admin_port: u16,
    /// Forward-auth config, present only when gateway sign-in is on. When set,
    /// the listener(s) flagged `ext_authz` (the shared one only) gate every route behind it.
    auth: Option<AuthCtx<'a>>,
    /// The TLS listener for emulated public hostnames, when any module declares them.
    tls: Option<TlsCtx<'a>>,
    /// The shared listener's certificate, when forward-auth serves it over HTTPS.
    secure: Option<SecureCtx>,
    /// The authorization policy engine's cluster, when a policy module is selected. Checked
    /// after forward-auth on the listener(s) flagged `ext_authz`.
    policy: Option<PolicyCtx<'a>>,
}

#[derive(Serialize)]
struct PolicyCtx<'a> {
    cluster: &'a str,
}

/// The TLS listener's template context: its port, the certificate it presents, and one
/// virtual host per upstream cluster.
#[derive(Serialize)]
struct TlsCtx<'a> {
    port: u16,
    cert_chain: String,
    private_key: String,
    virtual_hosts: Vec<TlsVhostCtx<'a>>,
}

/// The certificate the shared listener presents under forward-auth.
#[derive(Serialize)]
struct SecureCtx {
    cert_chain: String,
    private_key: String,
}

#[derive(Serialize)]
struct TlsVhostCtx<'a> {
    cluster: &'a str,
    hosts: &'a [String],
}

/// The forward-auth template context: the `ext_authz` upstream and the trusted identity
/// headers (stripped on ingress, re-added from the auth response).
#[derive(Serialize)]
struct AuthCtx<'a> {
    cluster: &'a str,
    port: u16,
    path_prefix: &'a str,
    /// The gateway path the provider's login portal is routed at (ext_authz disabled on it).
    portal_prefix: &'a str,
    identity_headers: &'a [String],
}

/// One Envoy listener: the port it binds and its route table.
#[derive(Serialize)]
struct ListenerCtx<'a> {
    name: String,
    internal_port: u16,
    routes: Vec<RouteCtx<'a>>,
    /// Whether this listener gates its routes behind the `ext_authz` filter (and strips the
    /// trusted identity headers on ingress). Only the shared listener — the one carrying the
    /// API/UI routes — is gated; dedicated listeners (gatewayed object stores, fixed-path UIs)
    /// are deliberately left open, so resource backends are never behind auth.
    ext_authz: bool,
}

#[derive(Serialize)]
struct RouteCtx<'a> {
    prefix: &'a str,
    /// [`prefix`](Self::prefix) as a literal RE2 pattern, for the `regex_rewrite` match.
    prefix_regex: String,
    cluster: &'a str,
    rewrite: Option<&'a str>,
    /// Hold the request open with no route timeout (SSE, streamed model output).
    streaming: bool,
    /// Drop caller credentials before forwarding (the upstream holds its own).
    strip_client_credentials: bool,
}

/// Escape `literal` so RE2 matches it verbatim, in a form that is also valid inside a YAML
/// double-quoted scalar: path-safe characters pass through, anything else becomes a
/// `\\x{HH}` escape (YAML turns the doubled backslash into the single one RE2 reads). A prefix
/// like `/api/2.1/unity-catalog` would otherwise let its `.` match any character.
fn re2_literal(literal: &str) -> String {
    literal
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '/' | '-' | '_' | '~') {
                c.to_string()
            } else {
                format!("\\\\x{{{:X}}}", c as u32)
            }
        })
        .collect()
}

#[derive(Serialize)]
struct ClusterCtx<'a> {
    name: &'a str,
    host: &'a str,
    port: u16,
    http2: bool,
}

/// Render the Envoy bootstrap config for the plan's gateway.
///
/// Reproduces the Databricks-edge shape: the shared, path-multiplexed listener on the
/// gateway's internal port (each [`GatewayRoute`](crate::GatewayRoute) in order, with a
/// `regex_rewrite` block only when the route has a rewrite), then one dedicated listener per
/// fixed-path UI / gatewayed backend (serving `/` to its own cluster on its own port), the
/// upstream clusters, and the admin endpoint. The [`GatewayConfig`] is the single source of
/// truth — the app catch-all route and cluster, when present, are already in it (the planner
/// puts them there from [`PlanCtx::app`](crate::PlanCtx::app)).
pub fn render_envoy(gateway: &GatewayConfig) -> String {
    // Each planner listener becomes an Envoy listener. The shared listener is first (it carries
    // the API/UI routes and, when present, the app catch-all the planner already appended);
    // dedicated listeners (their own port, a single `/` route) follow. An empty gateway still
    // emits a valid, route-less shared listener.
    let mut listeners: Vec<ListenerCtx> = gateway
        .listeners
        .iter()
        .enumerate()
        .map(|(i, l)| ListenerCtx {
            name: if i == 0 {
                "gateway".to_string()
            } else {
                format!("dedicated_{}", l.host_port)
            },
            internal_port: l.internal_port,
            routes: l
                .routes
                .iter()
                .map(|r| RouteCtx {
                    prefix: &r.prefix,
                    prefix_regex: re2_literal(&r.prefix),
                    cluster: &r.cluster,
                    rewrite: r.rewrite.as_deref(),
                    streaming: r.options.streaming,
                    strip_client_credentials: r.options.strip_client_credentials,
                })
                .collect(),
            // The shared listener is gated: it carries the API/UI routes. Dedicated listeners
            // front resource backends (gatewayed object stores) or UIs with their own login, and
            // stay open unless their endpoint asks for the sign-in.
            ext_authz: (i == 0 || l.gated) && gateway.auth.is_some(),
        })
        .collect();
    if listeners.is_empty() {
        // No listeners at all (empty plan) — emit a bare shared listener so the config is
        // still valid Envoy.
        listeners.push(ListenerCtx {
            name: "gateway".to_string(),
            internal_port: 10000,
            routes: Vec::new(),
            ext_authz: gateway.auth.is_some(),
        });
    }

    let clusters: Vec<ClusterCtx> = gateway
        .clusters
        .iter()
        .map(|c| ClusterCtx {
            name: &c.name,
            host: &c.host,
            port: c.port,
            http2: c.http2,
        })
        .collect();

    let ctx = EnvoyCtx {
        listeners,
        clusters,
        admin_port: gateway.admin_port,
        auth: gateway.auth.as_ref().map(|a| AuthCtx {
            cluster: &a.cluster,
            port: a.port,
            path_prefix: &a.path_prefix,
            portal_prefix: &a.portal_prefix,
            identity_headers: &a.identity_headers,
        }),
        policy: gateway.policy.as_ref().map(|p| PolicyCtx {
            cluster: &p.cluster,
        }),
        secure: gateway.secure.as_ref().map(|s| SecureCtx {
            cert_chain: format!("{}/leaf.pem", s.trust.mount_path),
            private_key: format!("{}/leaf-key.pem", s.trust.mount_path),
        }),
        tls: gateway.tls.as_ref().map(|t| TlsCtx {
            port: t.port,
            cert_chain: format!("{}/leaf.pem", t.trust.mount_path),
            private_key: format!("{}/leaf-key.pem", t.trust.mount_path),
            virtual_hosts: t
                .virtual_hosts
                .iter()
                .map(|v| TlsVhostCtx {
                    cluster: &v.cluster,
                    hosts: &v.hosts,
                })
                .collect(),
        }),
    };

    let mut env = minijinja::Environment::new();
    env.set_keep_trailing_newline(true);
    env.add_template("envoy", ENVOY_TEMPLATE)
        .expect("the embedded Envoy template is valid");
    env.get_template("envoy")
        .expect("the Envoy template was just added")
        .render(&ctx)
        .expect("rendering the Envoy template with a valid context cannot fail")
}
