//! The unified platform-surface invariant ([`SurfaceMode`]).
//!
//! A Lakehouse architecture exposes its platform services to clients through *one*
//! surface. That invariant holds across deployment shapes; only *what* the surface
//! is varies. This type names the variants so the desktop case is a recognized
//! shape of the same invariant rather than a fork in the consuming code.

use serde::{Deserialize, Serialize};

/// How an environment exposes its single platform surface to clients.
///
/// The invariant — *exactly one platform surface* — is constant; [`SurfaceMode`]
/// records which shape realizes it. It is a descriptor the resolver and validators
/// read, not a place to encode routing/auth policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceMode {
    /// The standard shape: platform services are containers fronted by the
    /// gateway, and clients reach the surface at the gateway's published port.
    /// This is the frontend-app deployment.
    Gatewayed,
    /// The desktop variant: the platform service runs
    /// [`InProcess`](crate::Placement::InProcess), so it is not *behind* the
    /// gateway — yet the app still presents the one platform surface to its UI via
    /// in-process dispatch. A recognized variant of the same invariant, not an
    /// exception to it.
    InProcessSurface,
}
