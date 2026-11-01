//! Topology and addressing for Lakehouse reference-architecture environments.
//!
//! Which service reaches which, and at what address, depends on where each one runs (in the
//! same process, on the host, or in a container) and on whether the traffic goes through the
//! gateway. This crate models those rules once, so callers don't recompute them.
//!
//! # The model, in six pieces
//!
//! - [`Role`] / [`ServiceSpec`] — a service declares the *role* it fills
//!   (`data_catalog`, `object_store`, `gateway`, …) independent of *which*
//!   implementation fills it. The role set is open, and implementation names
//!   (Unity Catalog, Azurite, …) are catalog data, so a new catalog or a
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
//! # The crate's shape: six phases, six module groups
//!
//! The public surface mirrors the phases an environment goes through, and the
//! source is grouped to match:
//!
//! - [`mod@model`] — the vocabulary the rest is described in ([`Role`], [`Endpoint`],
//!   [`Placement`], [`Connection`], …): pure data, no logic.
//! - [`mod@catalog`] — the [`Catalog`] of selectable [`Module`]s.
//! - [`mod@plan`] — the producer: a [`Selection`] against a [`Catalog`] resolves to a
//!   fully-assigned, target-neutral environment (no rendered text).
//! - [`mod@render`] — pure string rendering of a plan: the Docker Compose target
//!   ([`render::compose`]), the shared Envoy bootstrap, and the write-ready output.
//! - [`mod@target`] — the [`DeployTarget`]s an environment can run on (Docker Compose today),
//!   each turning a plan into its own files.
//! - [`mod@address`] — resolving the URL to reach a service from a given [`Vantage`].
//!
//! # Purity: rendering is pure, disk I/O is opt-in
//!
//! Planning and rendering touch no filesystem and spawn no processes. The render phase
//! produces **strings and relative filenames in memory** ([`Artifacts`], [`RenderOutput`],
//! [`RenderFile`]), and the crate never writes them. Persisting those strings to disk (or, in
//! a browser, visualizing them without persisting) is the **consumer's** job. Rendering is
//! always available and not feature-gated, so the rendering path (MiniJinja included) compiles
//! to `wasm32`. The two features that do touch disk are `std-io` (`MaterializedOutput::write_to`
//! and the manifest's `read_from`/`write_to`) and `catalog` (loading `module.yaml` manifests).
//!
//! Runtime facts the resolver needs — a dynamically-allocated catalog port, the
//! gateway's host-published port — are supplied at plan time. Catalog loading from
//! on-disk `module.yaml` manifests is the one genuinely I/O-shaped concern and stays
//! behind the `catalog` feature so pure consumers need not pull `serde_yaml`.
//!
//! # Persisting choices and re-planning
//!
//! [`EnvManifest`] persists a user's environment choices (the [`Selection`] plus the
//! environment-level [`PlanCtx`]) as TOML and re-creates the same [`Plan`] from them, so the
//! **gateway host port and each service's route stay stable when the selection is edited** (the
//! shared listener is stable by construction; see [`mod@manifest`]). [`layout_report`] renders a
//! plan's gateway layout as a terse Markdown summary. Both produce and consume strings only.

pub mod address;
pub mod catalog;
pub mod manifest;
pub mod model;
pub mod plan;
pub mod projection;
pub mod render;
pub mod report;
pub mod target;
pub mod workspace;

pub use model::connection::{
    Connection, ConnectionField, ConnectionTemplate, ObjectStoreCredential, TlsTrust,
};
pub use model::endpoint::{Endpoint, Rewrite, RouteIntent, RouteOptions, Scheme};
pub use model::placement::{Placement, Vantage};
pub use model::role::{Role, ServiceSpec};

pub use catalog::credentials::{
    LOCAL_CREDENTIALS, LocalCredential, LocalCredentialError, LocalCredentials,
};
pub use catalog::deploy::{DeployCtx, DeployPhase, DeployStepSpec, RenderedStep};
pub use catalog::module::{
    ClientNotebook, ClientValue, ConnectionBinding, DataModule, ExtraResource, GatewayTls,
    HelperKind, HelperService, ImpersonatedHost, Knob, KnobKind, KnobRejection, MeshNetwork,
    Module, ModuleId, PortDecl, PortMapping, Provides, PythonClient, Readiness, ReadyGate,
    RenderCtx, RenderSpec, ResolvedKnobs, ResourceDemand, StackFacts, TemplateError, UrlPart,
    is_model_ref,
};
pub use catalog::{
    Catalog, DATA_ROOT_DEFAULT, DATA_ROOT_VAR, HOST_PORT_VAR_PREFIX, baseline_catalog,
    baseline_catalog_with, baseline_selection,
};

pub use plan::resolve::{Edge, ExtraEdges, ResolveError, ResolvedGraph, resolve, resolve_with};
pub use plan::routing::{AssignedRoute, Listener, RoutePlan};
pub use plan::{
    AppUpstream, AuthConfig, ClientSelection, ClusterConfig, ENVOY_AUTH_KNOB,
    ENVOY_AUTH_KNOB_LEGACY, EXT_AUTHZ_PATH_EXTRA, GatewayConfig, GatewayRoute, GatewaySettings,
    HostedConnection, HostedKind, HostedResource, HostedStepView, HostedVendor, ListenerConfig,
    LocalSecrets, ModuleBinding, PYTHON_CLIENT_BASE_DEPENDENCIES, Plan, PlanCtx, PlanError,
    PolicyConfig, PythonWorkspace, Secrets, SecureOrigin, Selection, TlsListenerConfig,
    TlsVirtualHost, is_valid_resource_name, resource_name_rule,
};

pub use render::compose::{
    Artifacts, ComposeInclude, ConfigDecl, HeadFile, SecretDecl, render_all, render_compose,
    render_modules,
};
pub use render::deploy::{
    DEPLOY_README_PATH, DEPLOY_SCRIPTS_DIR, DEPLOY_SHELL_SETUP, DeployScript, DeployStep,
    SECRETS_ENV_PATH, render_readme, render_steps,
};
pub use render::envoy::render_envoy;
pub use render::output::{
    CLIENT_CONTAINER_ENV_PATH, CLIENT_HOST_ENV_PATH, MaterializedOutput, OutputFile,
};
pub use render::python_client::{PYTHON_CLIENT_DIR, python_client_files};
pub use render::{InjectedEnv, RenderError, RenderFile, RenderOutput};

pub use target::{Compose, DeployTarget, TargetId, builtin_targets};

pub use address::{AddressError, ServiceRef};

pub use manifest::{CatalogSource, EnvManifest, ManifestError, pin_dedicated_ports};

pub use workspace::{
    MAX_SLOT, PortConflict, SLOT_PORT_STRIDE, WORKSPACE_FILE, WorkspaceEnv, WorkspaceError,
    WorkspaceManifest, port_conflicts,
};

pub use report::layout_report;

pub use projection::graph_mermaid;
