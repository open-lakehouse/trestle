//! The environment **topology & addressing** framework for Lakehouse
//! reference-architecture environments.
//!
//! A Lakehouse dev environment is a set of services — a catalog, an object store,
//! a gateway, lineage, query engines — wired together. *Which* service reaches
//! *which*, and at *what address*, depends on where each one runs relative to the
//! other: in the same process, on the host, or in a container, and whether the
//! traffic goes through the gateway or direct. Computed by hand at each call site
//! those rules duplicate and drift; this crate makes them one tested model.
//!
//! # The model, in six pieces
//!
//! - [`Role`] / [`ServiceSpec`] — a service declares the *role* it fills
//!   (`data_catalog`, `object_store`, `gateway`, …) independent of *which*
//!   implementation fills it. The role set is open, and implementation names
//!   (Unity Catalog, Azurite, …) are *data*, never types — so a new catalog or a
//!   hybrid drops in without a framework change.
//! - [`Placement`] / [`Vantage`] — where a service runs and where a caller sits.
//! - [`Endpoint`] / [`RouteIntent`] — what a service offers (port, scheme) and its
//!   gateway *intent* — [`Api`](RouteIntent::Api),
//!   [`UiPrefixable`](RouteIntent::UiPrefixable), or [`UiFixed`](RouteIntent::UiFixed).
//!   The module declares intent only; it never picks its own prefix, nor describes
//!   *how* a base path is applied (that is the template's job).
//! - [`RoutePlan`] / [`AssignedRoute`] — the coordinator's per-endpoint routing
//!   decisions (prefix, rewrite, [`Listener`], chosen base path), assigned across
//!   all modules so paths don't collide. This is where prefixes are decided.
//! - [`RenderOutput`] / [`RenderFile`] — the planner↔template handshake: what a
//!   module's render produces (a compose fragment plus zero or more mountable
//!   files), with planner-decided values injected via compose env-var substitution.
//! - [`address`] — the single pure function that turns a `(from, to, endpoint,
//!   plan, ctx)` tuple into one concrete [`url::Url`], routing through the gateway
//!   when the plan assigns a route; [`address_direct`] is the explicit escape hatch.
//! - [`SurfaceMode`] — the "one unified platform surface" Lakehouse invariant,
//!   with the in-process desktop variant expressed in-model.
//!
//! # Purity
//!
//! The model and the resolver are pure: no I/O, no process spawning, no
//! templating. Runtime facts the resolver needs — a dynamically-allocated catalog
//! port, the gateway's host-published port — are passed in via [`TopologyCtx`].
//! Catalog loading (embedding a baseline, merging on-disk overlays) is a separate,
//! feature-gated concern so pure consumers stay free of those dependencies.

mod endpoint;
mod placement;
mod plan;
mod render;
mod resolve;
mod role;
mod surface;

pub use endpoint::{Endpoint, RouteIntent, Scheme};
pub use placement::{Placement, Vantage};
pub use plan::{AssignedRoute, Listener, RoutePlan};
pub use render::{InjectedEnv, RenderFile, RenderOutput};
pub use resolve::{AddressError, TopologyCtx, address, address_direct};
pub use role::{Role, ServiceSpec};
pub use surface::SurfaceMode;
