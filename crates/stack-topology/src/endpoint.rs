//! What a service offers ([`Endpoint`]), declared once and free of any caller's
//! vantage — and free of any *routing* decision.
//!
//! An endpoint names a port the service listens on, the scheme to speak to it, and
//! its [`RouteIntent`] — whether it should be fronted by the gateway, and if so
//! what *shape* it is (API, prefixable UI, fixed-path UI). The
//! [`address`](crate::address) resolver combines an endpoint with the caller's
//! [`Vantage`](crate::Vantage), the callee's [`Placement`](crate::Placement), and
//! the coordinator's [`RoutePlan`](crate::RoutePlan) to produce a concrete URL.
//!
//! # Module declares intent; the coordinator assigns the route
//!
//! A module (this catalog entry) declares only its *intrinsic*, planner-independent
//! facts. It does **not** pick its own gateway prefix, rewrite, or listener: those
//! are assigned by the coordinator/planner when the full environment is assembled,
//! because only the planner sees every module at once and can avoid path collisions
//! (two modules wanting `/api`, two UIs wanting `/`). The planner's choices live in
//! a [`RoutePlan`](crate::RoutePlan); this type carries only intent.
//!
//! The platform's posture is *everything that can be, goes through the gateway* —
//! that one unified front is where authn/authz and the rest of the cross-cutting
//! concerns are configured uniformly. How a service can be fronted depends on what
//! it serves ([`RouteIntent`]):
//!
//! - An **API** ([`RouteIntent::Api`]) is path-agnostic: the planner can mount it
//!   under any prefix and rewrite to the service root harmlessly.
//! - A **prefixable UI** ([`RouteIntent::UiPrefixable`]) emits self-referential
//!   links, so it can only be cleanly fronted when the *service itself* serves
//!   under the planner's chosen base path. The planner announces that base path as
//!   a value (on the [`AssignedRoute`](crate::AssignedRoute)); *how* the service is
//!   told it — a CLI flag like MLflow's `--static-prefix`, an env var, a line in a
//!   mounted config file — is a service-specific mechanic the **template** owns, not
//!   something this model encodes.
//! - A **fixed-path UI** ([`RouteIntent::UiFixed`]) cannot take a base path at all,
//!   so the planner must give it its own listener / external port rather than
//!   path-multiplex it on the shared one.

use serde::{Deserialize, Serialize};

/// The wire scheme an endpoint speaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scheme {
    Http,
    Https,
    /// gRPC over HTTP/2. Rendered with an `http://` URL scheme (gRPC clients take a
    /// plain `host:port` target); the variant is kept distinct so callers and the
    /// gateway can configure HTTP/2 explicitly.
    Grpc,
    /// A raw TCP endpoint (e.g. a database port). Rendered as `tcp://host:port`.
    Tcp,
}

impl Scheme {
    /// The URL scheme string used when rendering an address for this endpoint.
    pub fn url_scheme(self) -> &'static str {
        match self {
            Scheme::Http | Scheme::Grpc => "http",
            Scheme::Https => "https",
            Scheme::Tcp => "tcp",
        }
    }
}

/// What an endpoint is *for*, as far as gateway fronting goes — the intrinsic shape
/// the module declares. The coordinator reads this to assign an actual route (see
/// [`RoutePlan`](crate::RoutePlan)); the module never names a prefix itself, and it
/// never describes *how* a base path is applied (that is the template's job).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum RouteIntent {
    /// Not part of the platform surface — reached only directly (e.g. a database
    /// port, an inter-service-only endpoint). The gateway never fronts it.
    Internal,
    /// A path-agnostic API. The planner may mount it under any prefix and rewrite
    /// to the service root without breaking it.
    Api,
    /// A UI that emits self-referential links and *can* serve under a base path.
    /// The planner picks the base path and announces it (on the
    /// [`AssignedRoute`](crate::AssignedRoute)); the template applies it however the
    /// service requires (flag, env var, mounted config file). This model says only
    /// that the UI is prefixable, not how the prefix is wired.
    UiPrefixable,
    /// A UI that cannot take a base path. The planner must give it its own
    /// listener / external port rather than path-multiplex it.
    UiFixed,
    /// A whole-service backend that must be fronted at the *origin* of its own
    /// external port — not path-multiplexed under a shared-listener prefix. The
    /// planner gives it a dedicated listener serving `/` (like
    /// [`UiFixed`](RouteIntent::UiFixed)), but the intent is semantically distinct:
    /// this is for services whose clients construct URLs from the endpoint origin and
    /// would break under a path prefix — chiefly object stores (S3/Blob SDKs build
    /// `<endpoint>/<bucket>/<key>` and sign against the host), which therefore cannot
    /// sit behind a shared-listener prefix the way an [`Api`](RouteIntent::Api) can.
    Gatewayed,
}

impl RouteIntent {
    /// Whether the gateway can front this endpoint (everything except
    /// [`Internal`](RouteIntent::Internal)).
    pub fn is_surface(&self) -> bool {
        !matches!(self, RouteIntent::Internal)
    }

    /// Whether this endpoint serves a UI (prefixable or fixed-path).
    pub fn is_ui(&self) -> bool {
        matches!(self, RouteIntent::UiPrefixable | RouteIntent::UiFixed)
    }
}

/// A single network endpoint a service offers.
///
/// Carries both the **internal port** (what the service listens on, used for
/// in-process / host / container-direct vantages) and the optional **host port**
/// (the compose `ports:` mapping or a dynamically-allocated host port, used when a
/// host-side caller reaches a [`Host`](crate::Placement::Host) service or a
/// gateway directly). Conflating the two is the classic addressing bug; keeping
/// them distinct is deliberate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Endpoint {
    /// Stable endpoint identifier on its service (e.g. `"rest"`, `"s3"`,
    /// `"otlp_grpc"`, `"lineage"`). Unique within a service's endpoint list.
    pub id: String,
    /// The scheme to speak to this endpoint.
    pub scheme: Scheme,
    /// The port the service listens on (in its container, or on the host process
    /// for a host/in-process placement). Used for every vantage except a host-side
    /// caller that must hit a host-published port.
    pub internal_port: u16,
    /// The host-published port, when the service exposes one (compose `ports:`
    /// left-hand side, or a sidecar's bound host port). `None` for endpoints only
    /// reachable inside the compose network or in-process.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_port: Option<u16>,
    /// What this endpoint is for, as far as gateway fronting goes. The coordinator
    /// reads this to assign an actual route; the module never picks its own prefix.
    pub intent: RouteIntent,
    /// A path appended after the host/port (and, for a gateway reach, after the
    /// assigned route prefix) — e.g. a catalog API's `"/api/2.1/unity-catalog/"`.
    /// Empty for a bare `host:port` endpoint or a UI served at its base path.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub path: String,
}

impl Endpoint {
    /// Whether the gateway can front this endpoint (its [`RouteIntent`] is not
    /// [`Internal`](RouteIntent::Internal)).
    pub fn is_surface(&self) -> bool {
        self.intent.is_surface()
    }
}
