//! The coordinator's routing decisions ([`RoutePlan`]).
//!
//! A module declares only its intrinsic [`RouteIntent`](crate::RouteIntent); it
//! never picks its own gateway prefix, rewrite, or listener. Those are *assigned*
//! by the coordinator/planner when the full environment is assembled — that is the
//! only vantage from which path collisions across modules can be resolved (two
//! APIs wanting `/api`, two UIs wanting `/`). The planner's output is a
//! [`RoutePlan`]: a map from a service's endpoint to the [`AssignedRoute`] chosen
//! for it.
//!
//! [`address`](crate::address) consumes a `RoutePlan` alongside the
//! [`ServiceSpec`](crate::ServiceSpec): an endpoint with an assigned route resolves
//! through the gateway under the assigned prefix; an endpoint with none resolves
//! directly. The plan is also what the render step reads to wire a UI's chosen
//! `base_path` back into its fragment (via the endpoint's
//! [`PrefixKnob`](crate::PrefixKnob)).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Which gateway listener a route binds to — a coordinator decision.
///
/// The shared listener path-multiplexes most services behind one external port; it
/// is flexible enough (HTTP, WebSocket upgrades, …) for nearly everything. A
/// dedicated listener is what the planner assigns to a
/// [`UiFixed`](crate::RouteIntent::UiFixed) UI that cannot be served under a base
/// path and so must own its external port.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Listener {
    /// The shared, path-multiplexed gateway listener (the default external port).
    #[default]
    Shared,
    /// A dedicated listener on its own external port.
    Dedicated {
        /// The host-published external port for this listener.
        port: u16,
    },
}

/// One route the coordinator assigned to a service's endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssignedRoute {
    /// The client-facing path prefix the gateway matches for this endpoint, chosen
    /// by the planner to avoid collisions (e.g. `"/api/2.1/unity-catalog"`,
    /// `"/mlflow"`).
    pub prefix: String,
    /// The upstream path the gateway rewrites `prefix` to before forwarding.
    /// `None` forwards unchanged. Meaningful for an [`Api`](crate::RouteIntent::Api)
    /// endpoint; a prefixable UI is configured at `prefix` instead (see
    /// `base_path`), so it carries no divergent rewrite. Not applied to the
    /// client-facing address the resolver returns — carried for the gateway-config
    /// consumer to emit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rewrite: Option<String>,
    /// The listener this route binds to.
    #[serde(default)]
    pub listener: Listener,
    /// For a [`UiPrefixable`](crate::RouteIntent::UiPrefixable) endpoint, the base
    /// path the planner chose and that must be fed back into the service (via its
    /// [`PrefixKnob`](crate::PrefixKnob)) so its self-referential links resolve.
    /// Normally equals `prefix`. `None` for APIs and fixed-path UIs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_path: Option<String>,
}

/// The coordinator's full set of route assignments, keyed by `(service, endpoint)`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RoutePlan {
    routes: BTreeMap<(String, String), AssignedRoute>,
}

impl RoutePlan {
    /// An empty plan (nothing gatewayed — every endpoint resolves directly).
    pub fn new() -> Self {
        RoutePlan::default()
    }

    /// Assign `route` to `(service, endpoint)`, replacing any previous assignment.
    pub fn assign(
        &mut self,
        service: impl Into<String>,
        endpoint: impl Into<String>,
        route: AssignedRoute,
    ) {
        self.routes.insert((service.into(), endpoint.into()), route);
    }

    /// The route assigned to `(service, endpoint)`, if any.
    pub fn get(&self, service: &str, endpoint: &str) -> Option<&AssignedRoute> {
        // `(String, String): Borrow<(&str, &str)>` does not hold, so look up via a
        // borrowed-key map view. A small owned key here is fine — resolution is not
        // a hot path — and keeps the lookup O(log n) rather than scanning.
        self.routes
            .get(&(service.to_string(), endpoint.to_string()))
    }
}
