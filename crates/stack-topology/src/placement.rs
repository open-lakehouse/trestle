//! Where a service runs ([`Placement`]) and where a caller sits ([`Vantage`]).
//!
//! These two, together with the callee's [`Endpoint`](crate::Endpoint), are all
//! the [`address`](crate::address) resolver needs to pick the right host and port.
//! They are the distinction neither sibling tool names today: hydrofoil encodes it
//! by hand across several call sites, and trestle omits it entirely (assuming every
//! caller is on the host, which silently breaks in-container callers).

use serde::{Deserialize, Serialize};

/// Where a service physically runs. This is what a callee carries; the resolver
/// reads it (against the caller's [`Vantage`]) to decide the host part of an
/// address.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Placement {
    /// Runs inside the host process itself (e.g. an embedded query engine reached
    /// by direct in-process dispatch rather than over the network).
    InProcess,
    /// A host-bound process outside any container — a sidecar or an
    /// app-level shared service (e.g. a catalog server bound to `127.0.0.1`, a
    /// shared telemetry collector). Reached on `127.0.0.1`/`localhost` from the
    /// host and across the container boundary via `host.docker.internal`.
    Host,
    /// A container on the compose network, addressed by its compose service name
    /// (its DNS name on that network).
    Container {
        /// The compose service / DNS name (e.g. `"db"`, `"mlflow"`,
        /// `"marquez-api"`).
        service: String,
    },
}

/// Where the *caller* sits. The same callee resolves to different addresses
/// depending on this — the crux the resolver exists to centralize.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Vantage {
    /// Calling from inside the host process (e.g. the embedded engine wiring up
    /// its catalog / lineage endpoints).
    InProcess,
    /// Calling from a host-side process or the desktop UI — anything that resolves
    /// `localhost` to the host and reaches containers only via the gateway's
    /// published port.
    Host,
    /// Calling from inside a container on the compose network.
    Container,
}
