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
//! - [`ServiceRef`] — a service handle from a [`Plan`] ([`Plan::service`] /
//!   [`Plan::service_by_role`]); its [`address`](ServiceRef::address) turns a
//!   `(vantage, endpoint)` pair into one concrete [`url::Url`], routing through the gateway
//!   when the plan assigns a route ([`address_direct`](ServiceRef::address_direct) is the
//!   explicit escape hatch).
//!
//! # The crate's shape: five phases, five module groups
//!
//! The public surface mirrors the phases an environment goes through, and the
//! source is grouped to match:
//!
//! - [`mod@model`] — the vocabulary the rest is described in ([`Role`], [`Endpoint`],
//!   [`Placement`], [`Connection`], …): pure data, no logic.
//! - [`mod@catalog`] — the [`Catalog`] of selectable [`Module`]s.
//! - [`mod@plan`] — the producer: a [`Selection`] against a [`Catalog`] resolves to a
//!   fully-assigned environment.
//! - [`mod@render`] — pure string rendering of the compose / Envoy / `.env` artifacts.
//! - [`mod@address`] — resolving the URL to reach a service from a given [`Vantage`].
//!
//! # Purity: rendering is pure, only I/O lives in the consumer
//!
//! The whole crate is pure: no filesystem access, no process spawning. *Rendering*
//! is part of that purity — the render phase produces **strings and relative
//! filenames in memory** ([`Artifacts`], [`RenderOutput`], [`RenderFile`]); the crate
//! never writes them. Persisting those strings to disk (or, in a browser,
//! visualizing them without persisting) is the **consumer's** job. That is why
//! rendering is always available and not feature-gated: the only WASM-incompatible
//! step — disk I/O — is not in this crate at all, so the rendering path (MiniJinja
//! included) compiles to `wasm32` cleanly.
//!
//! Runtime facts the resolver needs — a dynamically-allocated catalog port, the
//! gateway's host-published port — are supplied at plan time. Catalog loading from
//! on-disk `module.yaml` manifests is the one genuinely I/O-shaped concern and stays
//! behind the `catalog` feature so pure consumers need not pull `serde_yaml`.

pub mod address;
pub mod catalog;
pub mod model;
pub mod plan;
pub mod render;

// --- model: the vocabulary types ---
pub use model::connection::{
    Connection, ConnectionField, ConnectionTemplate, ObjectStoreCredential,
};
pub use model::endpoint::{Endpoint, Rewrite, RouteIntent, Scheme};
pub use model::placement::{Placement, Vantage};
pub use model::role::{Role, ServiceSpec};

// --- catalog: the module set + how a module is defined ---
pub use catalog::module::{
    ConnectionBinding, DataModule, DepGate, DependsCondition, Knob, KnobKind, Module, ModuleId,
    PortDecl, PortMapping, Provides, RenderCtx, RenderError, RenderSpec, ResolvedKnobs,
    ResourceDemand,
};
pub use catalog::{
    Catalog, DATA_ROOT_DEFAULT, DATA_ROOT_VAR, baseline_catalog, baseline_selection,
};

// --- plan: the producer + the resolved environment ---
pub use plan::resolve::{Edge, ExtraEdges, ResolveError, ResolvedGraph, resolve, resolve_with};
pub use plan::routing::{AssignedRoute, Listener, RoutePlan};
pub use plan::{
    AppUpstream, AuthConfig, ClusterConfig, ComposeInclude, ConfigDecl, ENVOY_AUTH_KNOB,
    EXT_AUTHZ_PATH_EXTRA, GatewayConfig, GatewayRoute, HeadFile, ListenerConfig, Plan, PlanCtx,
    PlanError, Selection,
};

// --- render: the planner↔template handshake + the stack artifacts ---
pub use render::artifacts::{Artifacts, render_all, render_compose, render_envoy};
pub use render::output::{MaterializedOutput, OutputFile};
pub use render::{InjectedEnv, RenderFile, RenderOutput};

// --- address: the addressing resolver ---
pub use address::{AddressError, ServiceRef};
