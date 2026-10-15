//! The inlined baseline catalog, built purely in Rust (no I/O, no YAML).
//!
//! These are the common local-Lakehouse modules — a gateway, a relational store, an
//! object store, experiment tracking (with an AI Gateway), a data catalog, a query engine,
//! a graph engine, tracing, and an agent platform — most of them transcribed from trestle's
//! `local-stack-*` components. They are the corpus the planner is validated against:
//! planning the default selection must re-derive the routes and materialize the
//! artifacts these components ship today (see the crate's golden tests).
//!
//! # How a module encodes its routing facts (without authoring routes)
//!
//! A module declares only *intent* on its endpoints; the planner assigns the actual
//! prefixes/rewrites. The facts the planner needs to reproduce a service-specific rewrite
//! are typed fields on the services a module emits:
//!
//! - [`ServiceSpec::base_path`](crate::ServiceSpec::base_path) — where the service serves
//!   itself (e.g. MLflow under `/mlflow`). The planner derives an API rewrite as
//!   `base_path + client_path`, and uses it as a prefixable UI's chosen base path.
//! - [`Endpoint::mount_prefix`](crate::Endpoint::mount_prefix) — an `Api` endpoint's client
//!   mount prefix (the endpoint's own `path` stays empty so the resolver does not double it).
//! - [`Endpoint::rewrite`](crate::Endpoint::rewrite) — the [`Rewrite`](crate::Rewrite)
//!   policy: `Inherit` (derive from `base_path`), `Passthrough` (forward unchanged, e.g.
//!   Jaeger's `/v1/traces`), or `To(path)` (an explicit upstream path, e.g. MLflow's
//!   Databricks-shaped `/api/2.0/otel` onto its `/mlflow/v1/traces`).
//! - [`Endpoint::route`](crate::Endpoint::route) — [`RouteOptions`](crate::RouteOptions) for
//!   how the gateway treats requests on the route: `streaming` (no request timeout) and
//!   `strip_client_credentials` (e.g. MLflow's AI Gateway, which holds the provider keys).
//!
//! Most modules need no logic and are built as [`DataModule`](crate::DataModule)s (their
//! static services are returned verbatim). A module that varies its topology with a knob —
//! e.g. [`headwaters`], which drops its UI route when its UI knob is off — is a hand-written
//! type implementing [`Module`](crate::Module).
//!
//! # Compose fragments
//!
//! Each module is a self-contained directory under the crate-root `templates/` tree (a
//! sibling of `src/`): a selectable module's compose `services:` snippet lives at
//! `templates/modules/<name>/compose.yaml.jinja`, and the gateway's templates live together
//! under `templates/gateway/` — `compose.yaml.jinja` for the envoy module fragment,
//! `bootstrap.yaml.jinja` for the aggregated Envoy config rendered by [`crate::render_envoy`],
//! and the `authelia.*.jinja` faces of the bundled forward-auth provider the gateway's
//! sign-in setting pulls in. Each snippet is embedded via `include_str!` (relative paths
//! reach out of `src/catalog/` as `../../templates/…`) and carried on the module's
//! [`RenderSpec`]. The `.jinja` suffix marks a MiniJinja template: every fragment is rendered
//! against the typed [`RenderCtx`](crate::RenderCtx), so it reads plan-resolved values directly
//! (`{{ env.DATA_ROOT }}`, `{{ connections.object_store.0.credential.access_key_id }}`) and
//! branches on a resolved [`Connection`](crate::Connection) where it must. Plan-time values are
//! rendered concrete; RustFS's bucket-init lines, for instance, iterate the provisioned
//! `objects` rather than splicing a planner-filled placeholder.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::Arc;

use super::Catalog;
use crate::catalog::images;
use crate::catalog::module::{
    ClientNotebook, ClientValue, ConnectionBinding, DataModule, ImpersonatedHost, Knob, KnobKind,
    Module, ModuleId, Provides, PythonClient, Readiness, ReadyGate, RenderSpec, ResolvedKnobs,
    ResourceDemand, StackFacts, UrlPart,
};
use crate::model::connection::{
    Connection, ConnectionField, ConnectionTemplate, ObjectStoreCredential,
};
use crate::model::endpoint::{Endpoint, Rewrite, Scheme};
use crate::model::placement::Placement;
use crate::model::role::{Role, ServiceSpec};
use crate::plan::hosted::HostedKind;
use crate::render::RenderFile;
use crate::target::TargetId;

/// The render-env key carrying the stack's root data directory, injected into *every*
/// module's render env (see [`PlanCtx::data_root`](crate::PlanCtx::data_root)).
///
/// A module that persists state across `compose down`/`up` mounts its data under
/// `{{ env.DATA_ROOT }}/<module>` by convention, rather than hard-coding a `./.data/...` path
/// relative to the compose file. The value is resolved at *plan time* (baked into the rendered
/// fragment, like [`BASE_PATH`]), so relocating the whole stack's persistence is a single
/// [`PlanCtx::data_root`](crate::PlanCtx::data_root) knob — no per-fragment edit. A module with
/// no durable state simply ignores it.
///
/// [`BASE_PATH`]: crate::InjectedEnv
pub const DATA_ROOT_VAR: &str = "DATA_ROOT";

/// The render-only variable carrying the gateway's public origin as a host-side browser
/// reaches it (`http://localhost:<gateway_host_port>`, or `https://<env>.localhost:<port>` with
/// forward-auth on), so a module that must know its own public URL (e.g. an auth portal's
/// redirect target) follows the configured gateway port instead of hard-coding one. Baked at
/// plan time, like [`DATA_ROOT_VAR`].
pub const GATEWAY_URL_VAR: &str = "GATEWAY_URL";

/// The render-only variable carrying the bare hostname of [`GATEWAY_URL_VAR`] (`localhost`, or
/// `<env>.localhost` with forward-auth on), e.g. for a cookie domain. Baked at plan time.
pub const GATEWAY_HOST_VAR: &str = "GATEWAY_HOST";

/// The render-only variable carrying the gateway's origin as an in-network caller reaches it
/// (`http://<gateway_service>:<gateway_internal_port>`, or `http://<env>.internal:<port>` with
/// forward-auth on): the container-vantage counterpart of [`GATEWAY_URL_VAR`], e.g. for a
/// service that must accept the `Host` its in-network callers send. Baked at plan time, like
/// [`DATA_ROOT_VAR`].
pub const GATEWAY_INTERNAL_URL_VAR: &str = "GATEWAY_INTERNAL_URL";

/// The render-only variable carrying the bare hostname of [`GATEWAY_INTERNAL_URL_VAR`].
pub const GATEWAY_INTERNAL_HOST_VAR: &str = "GATEWAY_INTERNAL_HOST";

/// The render-only variable saying whether the gateway authenticates the requests it fronts:
/// `"true"` when its forward-auth knob is on and an auth provider is wired, else `"false"`.
/// A fragment branches on it to trust the identity the gateway asserts (e.g. `Remote-Email`)
/// or to run single-user. It mirrors [`StackFacts::gateway_auth`](crate::StackFacts), which
/// the module's [`services_in`](crate::Module::services_in) sees, so both agree.
pub const GATEWAY_AUTH_VAR: &str = "GATEWAY_AUTH";

/// The render-only variable listing every client-environment key the stack defines (the
/// selected modules' [`Provides::client_env`](crate::Provides::client_env) keys, sorted and
/// comma-joined), so a module that launches client processes (an agent runner) can forward
/// exactly the stack's contract to them without naming other modules' variables.
pub const CLIENT_ENV_KEYS_VAR: &str = "CLIENT_ENV_KEYS";

/// The default value injected for [`DATA_ROOT_VAR`]: `./.data`, relative to the compose
/// file's directory — matching where the fragments persisted data before the root was
/// centralized.
pub const DATA_ROOT_DEFAULT: &str = "./.data";

/// The inlined baseline catalog: all common local-Lakehouse modules.
///
/// The `object_store` role has two in-stack providers (RustFS and Azurite) and two hosted ones
/// (`aws-s3`, `azure-storage`); the catalog default is RustFS, so the baseline plans the S3
/// wiring out of the box. `relational_db` is the in-stack `postgres` or `hosted-postgres`. An
/// environment uses one provider per role: selecting another one (or preferring it via
/// [`PlanCtx::provider_preference`](crate::PlanCtx::provider_preference)) replaces the default.
pub fn baseline_catalog() -> Catalog {
    Catalog::from_modules([
        envoy(),
        authelia(),
        postgres(),
        rustfs(),
        azurite(),
        hosted_postgres(),
        aws_s3(),
        azure_storage(),
        mlflow(),
        unity_catalog(),
        jaeger(),
        headwaters(),
        omnigent(),
        marimo(),
        puppygraph(),
        opa(),
    ] as [Arc<dyn Module>; 16])
    .with_default_provider(Role::OBJECT_STORE, "rustfs")
    .with_default_provider(Role::RELATIONAL_DB, "postgres")
    // No coordinate contracts: a provider vends a typed `Connection`, whose variant fields
    // are all mandatory, so completeness is a compile-time guarantee rather than a runtime
    // check.
}

/// `hosted-postgres` — a managed PostgreSQL server (Amazon RDS, Azure Database for
/// PostgreSQL, Neon, …) in place of the in-stack `postgres`. Consumers connect to it directly;
/// the databases they demand must already exist.
fn hosted_postgres() -> Arc<dyn Module> {
    hosted_module(
        "hosted-postgres",
        "Hosted Postgres",
        "A managed PostgreSQL server (RDS, Neon, …); databases must already exist.",
        "metadata_db",
        HostedKind::Postgres,
        ServiceSpec {
            name: "db".into(),
            role: Role::relational_db(),
            placement: hosted_placement(),
            endpoints: vec![Endpoint::internal("sql", Scheme::Tcp, 5432, None)],
            depends_on: vec![],
            base_path: String::new(),
        },
    )
}

/// `aws-s3` — Amazon S3 (or an S3-compatible store at an explicit endpoint) in place of an
/// in-stack object store. The buckets consumers demand must already exist.
fn aws_s3() -> Arc<dyn Module> {
    hosted_module(
        "aws-s3",
        "Amazon S3",
        "Amazon S3 or an S3-compatible store; buckets must already exist.",
        "storage",
        HostedKind::S3,
        ServiceSpec {
            name: "s3".into(),
            role: Role::object_store(),
            placement: hosted_placement(),
            endpoints: vec![Endpoint::internal("s3", Scheme::Https, 443, None)],
            depends_on: vec![],
            base_path: String::new(),
        },
    )
}

/// `azure-storage` — an Azure storage account (Blob / ADLS Gen2) in place of an in-stack
/// object store. The containers consumers demand must already exist.
fn azure_storage() -> Arc<dyn Module> {
    hosted_module(
        "azure-storage",
        "Azure Storage",
        "An Azure storage account (Blob / ADLS Gen2); containers must already exist.",
        "storage",
        HostedKind::AzureBlob,
        ServiceSpec {
            name: "blob".into(),
            role: Role::object_store(),
            placement: hosted_placement(),
            endpoints: vec![Endpoint::internal("blob", Scheme::Https, 443, None)],
            depends_on: vec![],
            base_path: String::new(),
        },
    )
}

/// A hosted provider of `kind`'s role: one service, reached at the configured connection's
/// origin (the planner fills in the placement), and nothing to render. Its template is the
/// kind's default connection; the planner binds the configured one instead.
fn hosted_module(
    id: &str,
    display_name: &str,
    summary: &str,
    category: &str,
    kind: HostedKind,
    service: ServiceSpec,
) -> Arc<dyn Module> {
    let mut provides = Provides {
        hosted: Some(kind),
        ..Provides::default()
    };
    provides
        .resource_kinds
        .insert(kind.role().into(), kind.default_connection().template());
    Arc::new(DataModule {
        id: ModuleId::from(id),
        display_name: Some(display_name.into()),
        summary: Some(summary.into()),
        category: Some(category.into()),
        provider_of: Some(kind.role().into()),
        requires: vec![],
        conflicts_with: vec![],
        needs: vec![],
        service_specs: vec![service],
        provides,
        knobs: vec![],
        renders: BTreeMap::new(),
    })
}

/// A hosted service's placement before planning, which replaces the origin with the
/// configured connection's.
fn hosted_placement() -> Placement {
    Placement::External {
        origin: String::new(),
    }
}

/// The default lakehouse selection: the default category picks (a relational store and
/// an object store) and Unity Catalog as the lakehouse's data catalog. The gateway is not
/// listed: every plan includes it. Other modules (ml, query engine, observability, ...)
/// are opt-in.
pub fn baseline_selection() -> crate::plan::Selection {
    crate::plan::Selection::modules(["postgres", "rustfs", "unity-catalog"])
}

/// Helper: a container-placed service.
fn container(service: &str) -> Placement {
    Placement::Container {
        service: service.to_string(),
    }
}

/// Helper: a [`RenderSpec`] (MiniJinja) carrying just a compose fragment (no extra
/// files). Every module's fragment is rendered against the typed [`RenderCtx`](crate::RenderCtx),
/// so it reads plan-resolved values (`{{ env.X }}`, `{{ connections.* }}`) directly.
fn template(text: &str) -> RenderSpec {
    RenderSpec {
        fragment: text.to_string(),
        files: vec![],
    }
}

/// Helper: a [`RenderSpec`] carrying a fragment plus mounted config files. Each
/// file's `path` is module-relative (the planner roots it under `modules/<id>/`), and each
/// file template is rendered against the same [`RenderCtx`](crate::RenderCtx) as the fragment.
fn template_with_files(text: &str, files: Vec<RenderFile>) -> RenderSpec {
    RenderSpec {
        fragment: text.to_string(),
        files,
    }
}

/// A generated credential-bearing file loaded through Compose `env_file`.
fn sensitive_file(path: &str, contents: &str) -> RenderFile {
    RenderFile {
        path: path.into(),
        contents: contents.into(),
        alias: None,
        sensitive: true,
        secret_alias: None,
        at_root: false,
        preserve: false,
    }
}

/// A generated credential-bearing file declared as a top-level Compose secret.
fn secret_file(path: &str, alias: &str, contents: &str) -> RenderFile {
    RenderFile {
        path: path.into(),
        contents: contents.into(),
        alias: None,
        sensitive: true,
        secret_alias: Some(alias.into()),
        at_root: false,
        preserve: false,
    }
}

/// A generated, non-secret file declared as a top-level Compose config.
fn config_file(path: &str, alias: &str, contents: &str) -> RenderFile {
    RenderFile {
        path: path.into(),
        contents: contents.into(),
        alias: Some(alias.into()),
        sensitive: false,
        secret_alias: None,
        at_root: false,
        preserve: false,
    }
}

/// Helper: a string knob for a container image reference, defaulting to `default`.
///
/// `key` is the public snake_case identifier (typically `"image"` or `"init_image"`);
/// `legacy` is the old SCREAMING_SNAKE env-var-style name accepted as an override alias.
/// An operator-owned config file: written beside `compose.yaml` on first render and never
/// overwritten (`preserve`), so local edits survive re-renders.
fn operator_file(path: &str, alias: &str, contents: &str) -> RenderFile {
    RenderFile {
        at_root: true,
        preserve: true,
        ..config_file(path, alias, contents)
    }
}

fn image_knob(key: &str, title: &str, default: &str, legacy: &str) -> Knob {
    Knob {
        key: key.into(),
        title: Some(title.into()),
        kind: KnobKind::String,
        default: Some(default.into()),
        required: false,
        help: Some(
            "Container image reference (repository:tag or digest). Override to pin or \
             test a different release."
                .into(),
        ),
        aliases: vec![legacy.into()],
        advanced: true,
    }
}

/// `envoy` — the single-port gateway. It has no surface endpoints of its
/// own (it *is* the surface); its listening port is supplied to the planner via
/// [`PlanCtx`](crate::PlanCtx), not as a routed endpoint. Its rendered Envoy bootstrap config
/// is a planner-emitted artifact, not part of this fragment (which only declares the
/// container that mounts it).
///
/// When a module declares [`impersonated_hosts`](crate::Provides::impersonated_hosts), the
/// fragment also adds those hosts as network aliases and a one-shot that mints the local CA
/// the gateway's TLS listener presents.
///
/// Every environment runs it: the planner adds the `gateway`-role provider whether or not it
/// was selected. Sign-in and policy are [`GatewaySettings`](crate::GatewaySettings), realized
/// in the planner and the Envoy renderer (they pull in `authelia` / `opa` and emit the
/// `ext_authz` filters), so its only knobs are its images.
fn envoy() -> Arc<dyn Module> {
    let provides = Provides::default();
    Arc::new(DataModule {
        id: ModuleId::from("envoy"),
        display_name: Some("Envoy".into()),
        summary: Some("Single-port gateway, Databricks-shaped URL rewrites.".into()),
        category: Some("gateway".into()),
        provider_of: Some(Role::GATEWAY.into()),
        requires: vec![],
        conflicts_with: vec![],
        needs: vec![],
        service_specs: vec![ServiceSpec {
            name: "envoy".into(),
            role: Role::gateway(),
            placement: container("envoy"),
            endpoints: vec![Endpoint::internal("http", Scheme::Http, 10000, Some(9080))],
            depends_on: vec![],
            base_path: String::new(),
        }],
        provides,
        knobs: vec![
            image_knob("image", "Envoy image", images::ENVOY, "ENVOY_IMAGE"),
            image_knob(
                "certs_image",
                "Certificate minting image",
                images::CERTS,
                "GATEWAY_CERTS_IMAGE",
            ),
        ],
        // The certificate script renders only when the gateway emulates public hostnames
        // (`tls` is set); otherwise it is empty and dropped.
        renders: template_with_files(
            include_str!("../../templates/gateway/compose.yaml.jinja"),
            vec![config_file(
                "certs.sh",
                "gateway_certs_script",
                include_str!("../../templates/gateway/certs.sh.jinja"),
            )],
        )
        .for_compose(),
    })
}

/// `authelia` — the bundled forward-auth provider, pulled in by
/// [`GatewaySettings::auth`](crate::GatewaySettings::auth) (it is not part of the default
/// selection and rides in only when sign-in is on). The gateway waits for it to be healthy. Self-contained for the showcase: a file-based user
/// database and local, on-disk storage — no external database.
///
/// Its single endpoint is [`Internal`](crate::model::endpoint::RouteIntent::Internal): Envoy reaches
/// it in-network via the `authelia` cluster on port 9091 (the `ext_authz` target), and the
/// browser reaches its login portal through the gateway's redirect flow — so it needs no
/// host-published port. Two mounted files carry its config (`configuration.yml`) and the demo
/// user database (`users.yml`); both render against the typed [`RenderCtx`](crate::RenderCtx)
/// like every other fragment.
fn authelia() -> Arc<dyn Module> {
    // Authelia's HTTP `ext_authz` endpoint path. Declared here (catalog data), not in the
    // planner, so the planner stays free of implementation names: the planner reads it back via
    // [`EXT_AUTHZ_PATH_EXTRA`](crate::plan::EXT_AUTHZ_PATH_EXTRA) off the resolved provider.
    let mut provides = Provides::default();
    // Authelia serves its whole surface under `/authelia` (see configuration.yml), so the
    // ext_authz endpoint the gateway posts to is path-prefixed too.
    provides.extras.insert(
        crate::plan::EXT_AUTHZ_PATH_EXTRA.into(),
        "/authelia/api/authz/ext-authz/".into(),
    );
    // The gateway must not accept requests before it can check them.
    provides.ready_gate = Some(ReadyGate {
        service: "authelia".into(),
        readiness: Readiness::Healthy,
    });
    Arc::new(DataModule {
        id: ModuleId::from("authelia"),
        display_name: Some("Authelia".into()),
        summary: Some("Forward-auth single-sign-on for the gateway (file-based).".into()),
        category: Some("gateway".into()),
        provider_of: Some(Role::AUTH.into()),
        // No `requires`: the gateway is located by role, not id, so an id edge would break a
        // catalog with a differently-named gateway. Runtime ordering is the gateway fragment's
        // `depends_on: authelia` (Envoy waits on auth), which is the real dependency direction.
        requires: vec![],
        conflicts_with: vec![],
        needs: vec![],
        service_specs: vec![ServiceSpec {
            name: "authelia".into(),
            role: Role::auth(),
            placement: container("authelia"),
            // Internal: Envoy talks to it via the `authelia` cluster (ext_authz); the portal
            // UI is reached through the gateway's redirect, so no host port is published. The
            // planner derives the ext_authz cluster's host/port from this endpoint.
            endpoints: vec![Endpoint::internal("http", Scheme::Http, 9091, None)],
            depends_on: vec![],
            base_path: String::new(),
        }],
        provides,
        knobs: vec![image_knob(
            "image",
            "Authelia image",
            images::AUTHELIA,
            "AUTHELIA_IMAGE",
        )],
        renders: template_with_files(
            include_str!("../../templates/gateway/authelia.compose.yaml.jinja"),
            vec![
                RenderFile {
                    path: "configuration.yml".into(),
                    contents: include_str!(
                        "../../templates/gateway/authelia.configuration.yml.jinja"
                    )
                    .into(),
                    alias: Some("authelia_config".into()),
                    sensitive: false,
                    secret_alias: None,
                    at_root: false,
                    preserve: false,
                },
                // Operator-facing: lives next to `compose.yaml` so users can find and edit
                // local accounts. Seeded once (`preserve`) so later renders leave edits alone.
                RenderFile {
                    path: "users.yml".into(),
                    contents: include_str!("../../templates/gateway/authelia.users.yml.jinja")
                        .into(),
                    alias: Some("authelia_users".into()),
                    sensitive: false,
                    secret_alias: None,
                    at_root: true,
                    preserve: true,
                },
                sensitive_file(
                    ".env/authelia.env",
                    include_str!("../../templates/gateway/authelia.env.jinja"),
                ),
                secret_file(
                    "secrets/session_secret",
                    "authelia_session_secret",
                    "insecure_session_secret_change_me\n",
                ),
                secret_file(
                    "secrets/storage_encryption_key",
                    "authelia_storage_encryption_key",
                    "insecure_storage_encryption_key_change_me\n",
                ),
                secret_file(
                    "secrets/jwt_secret",
                    "authelia_jwt_secret",
                    "insecure_jwt_secret_change_me\n",
                ),
            ],
        )
        .for_compose(),
    })
}

/// `postgres` — the relational store. Internal-only (a database port is
/// never on the gateway surface).
fn postgres() -> Arc<dyn Module> {
    let mut provides = Provides::default();
    // The Postgres container reads these to initialize on first boot, so they stay in `.env`.
    // (Ports are written concretely in the fragment, so no `*_PORT` var is needed.)
    //
    // These are the *fixed* local-dev credentials, not an override surface: the connection URL
    // below bakes `postgres:postgres` concretely, and consumers (MLflow, UC) embed that
    // resolved connection. Editing `POSTGRES_USER`/`POSTGRES_PASSWORD` in `.env` would re-credential the
    // container but NOT repoint those consumers, so they must move together. (A typed, mutable
    // relational credential on the connection is the future home for making this configurable.)
    for (k, v) in [
        ("POSTGRES_USER", "postgres"),
        ("POSTGRES_PASSWORD", "postgres"),
        ("POSTGRES_DB", "postgres"),
    ] {
        provides.env_vars.insert(k.into(), v.into());
    }
    // Provisions `relational_db` resources; vends a typed relational connection. The
    // resource-kind key matches the service's role (`relational_db`) — one identity, so
    // role-exclusivity keys off the same name. The credential is folded into the URL,
    // concretely (matching the container's configured `postgres`/`postgres`) — no compose
    // `${VAR}` fallback for a consumer to resolve at run time.
    provides.resource_kinds.insert(
        Role::RELATIONAL_DB.into(),
        ConnectionTemplate(Connection::postgres(
            "db", 5432, "{name}", "postgres", "postgres",
        )),
    );
    // A consumer that demands `relational_db` should wait for the `db` service to be healthy.
    provides.ready_gate = Some(ReadyGate {
        service: "db".into(),
        readiness: Readiness::Healthy,
    });
    Arc::new(DataModule {
        id: ModuleId::from("postgres"),
        display_name: Some("Postgres".into()),
        summary: Some("Postgres 16; auto-creates DBs other modules declare.".into()),
        category: Some("metadata_db".into()),
        provider_of: Some(Role::RELATIONAL_DB.into()),
        requires: vec![],
        conflicts_with: vec![],
        needs: vec![],
        service_specs: vec![ServiceSpec {
            name: "db".into(),
            role: Role::relational_db(),
            placement: container("db"),
            endpoints: vec![Endpoint::internal("sql", Scheme::Tcp, 5432, Some(5432))],
            depends_on: vec![],
            base_path: String::new(),
        }],
        provides,
        knobs: vec![image_knob(
            "image",
            "Postgres image",
            images::POSTGRES,
            "POSTGRES_IMAGE",
        )],
        renders: template_with_files(
            include_str!("../../templates/modules/postgres/compose.yaml.jinja"),
            vec![
                RenderFile {
                    // Rendered from the databases the planner hands the provider via
                    // `RenderCtx.objects`; co-located under `modules/postgres/` and mounted into
                    // `/docker-entrypoint-initdb.d/` via the `postgres_init` config alias.
                    path: "init-databases.sh".into(),
                    contents: include_str!(
                        "../../templates/modules/postgres/init-databases.sh.jinja"
                    )
                    .into(),
                    alias: Some("postgres_init".into()),
                    sensitive: false,
                    secret_alias: None,
                    at_root: false,
                    preserve: false,
                },
                sensitive_file(
                    ".env/db.env",
                    include_str!("../../templates/modules/postgres/db.env.jinja"),
                ),
                secret_file(
                    "secrets/postgres_password",
                    "postgres_password",
                    "postgres\n",
                ),
            ],
        )
        .for_compose(),
    })
}

/// The RustFS root credentials. Local throwaway keys, used by the bucket init, the STS shim,
/// and every consumer of the store.
const RUSTFS_ACCESS_KEY: &str = "rustfs-root";
const RUSTFS_SECRET_KEY: &str = "rustfs-root-secret";

/// `rustfs` — the S3-compatible object store, with STS. Reached by SDKs through the
/// gateway's dedicated listener, like any gatewayed store. Its fragment iterates the buckets
/// it provisions (`objects`) to build the one-shot init.
///
/// RustFS also answers the real AWS S3/STS hostnames inside the compose network, through the
/// gateway's TLS listener ([`Provides::impersonated_hosts`]). That is for clients with no
/// endpoint override, like the Unity Catalog server: it always calls
/// `https://sts.<region>.amazonaws.com` to vend credentials, so the store must look like AWS
/// to it. An STS shim sits in front of RustFS because RustFS rejects the session policies such
/// clients send (see `templates/modules/rustfs/compose.yaml.jinja`).
fn rustfs() -> Arc<dyn Module> {
    let mut provides = Provides::default();
    // No env vars are declared here: the S3 credentials (`AWS_ACCESS_KEY_ID` /
    // `AWS_SECRET_ACCESS_KEY` / `AWS_DEFAULT_REGION`) are derived from the typed S3 credential
    // below (via `Connection::standard_env`), so they are stated once and enter `.env` only
    // when RustFS is the chosen object_store — no `AWS_*` leak under an Azure provider.
    provides.resource_kinds.insert(
        Role::OBJECT_STORE.into(),
        ConnectionTemplate(Connection::ObjectStore {
            uri: "s3://{name}".into(),
            bucket: "{name}".into(),
            // The in-network direct address. Because the `s3` endpoint is `Gatewayed`, the
            // planner rewrites this to the gateway origin (`http://<gateway>:<port>`) after
            // it allocates the store's dedicated listener, so consumers reach it via Envoy.
            endpoint: "http://rustfs:9000".into(),
            credential: ObjectStoreCredential::S3 {
                access_key_id: RUSTFS_ACCESS_KEY.into(),
                secret_access_key: RUSTFS_SECRET_KEY.into(),
                region: "us-east-1".into(),
                // RustFS ignores the role; any well-formed ARN lets a consumer AssumeRole.
                role_arn: Some("arn:aws:iam::000000000000:role/local-storage".into()),
            },
            tls_trust: None,
        }),
    );
    // A consumer that demands `object_store` should wait for the one-shot bucket init to
    // finish (the buckets it needs exist only after `rustfs-init` completes).
    provides.ready_gate = Some(ReadyGate {
        service: "rustfs-init".into(),
        readiness: Readiness::CompletedSuccessfully,
    });
    // Clients get the store's coordinates only — never its root keys. They read and write
    // with temporary credentials Unity Catalog vends for a table or volume.
    provides.client_env.insert(
        "AWS_ENDPOINT_URL_S3".into(),
        ClientValue::endpoint("rustfs", "s3", UrlPart::Full),
    );
    provides
        .client_env
        .insert("AWS_REGION".into(), ClientValue::literal("us-east-1"));
    // The AWS hostnames clients without an endpoint override call. Virtual-hosted bucket names
    // are listed per provisioned bucket (`{name}`).
    let sts = |host: &str| ImpersonatedHost {
        host: host.into(),
        service: "sts-shim".into(),
        port: 8080,
    };
    let s3 = |host: &str| ImpersonatedHost {
        host: host.into(),
        service: "rustfs".into(),
        port: 9000,
    };
    provides.impersonated_hosts = vec![
        sts("sts.amazonaws.com"),
        sts("sts.us-east-1.amazonaws.com"),
        s3("s3.amazonaws.com"),
        s3("s3.us-east-1.amazonaws.com"),
        s3("{name}.s3.amazonaws.com"),
        s3("{name}.s3.us-east-1.amazonaws.com"),
    ];
    Arc::new(DataModule {
        id: ModuleId::from("rustfs"),
        display_name: Some("RustFS".into()),
        summary: Some(
            "Self-hosted S3-compatible object store with STS; answers the AWS S3/STS \
             hostnames in-network."
                .into(),
        ),
        category: Some("storage".into()),
        provider_of: Some(Role::OBJECT_STORE.into()),
        requires: vec![],
        conflicts_with: vec![],
        needs: vec![],
        service_specs: vec![ServiceSpec {
            name: "rustfs".into(),
            role: Role::object_store(),
            placement: container("rustfs"),
            // No raw host port: the store is reached through the gateway on its own dedicated
            // listener (Gatewayed), not a direct compose `ports:` publish.
            endpoints: vec![Endpoint::gatewayed("s3", 9000)],
            depends_on: vec![],
            base_path: String::new(),
        }],
        provides,
        knobs: vec![
            image_knob("image", "RustFS image", images::RUSTFS, "RUSTFS_IMAGE"),
            image_knob(
                "init_image",
                "RustFS init image",
                images::RUSTFS_INIT,
                "RUSTFS_INIT_IMAGE",
            ),
            image_knob(
                "sts_shim_image",
                "STS shim image",
                images::STS_SHIM,
                "STS_SHIM_IMAGE",
            ),
        ],
        renders: template_with_files(
            include_str!("../../templates/modules/rustfs/compose.yaml.jinja"),
            vec![
                sensitive_file(".env/rustfs.env", &rustfs_env()),
                config_file(
                    "sts_shim.py",
                    "rustfs_sts_shim",
                    include_str!("../../templates/modules/rustfs/sts_shim.py"),
                ),
            ],
        )
        .for_compose(),
    })
}

/// The env file every RustFS service loads: the server's root keys, the same keys under the
/// names the bucket init (AWS CLI) and the STS shim read.
fn rustfs_env() -> String {
    format!(
        "RUSTFS_ACCESS_KEY={RUSTFS_ACCESS_KEY}\n\
         RUSTFS_SECRET_KEY={RUSTFS_SECRET_KEY}\n\
         AWS_ACCESS_KEY_ID={RUSTFS_ACCESS_KEY}\n\
         AWS_SECRET_ACCESS_KEY={RUSTFS_SECRET_KEY}\n\
         AWS_DEFAULT_REGION=us-east-1\n\
         STS_ACCESS_KEY={RUSTFS_ACCESS_KEY}\n\
         STS_SECRET_KEY={RUSTFS_SECRET_KEY}\n"
    )
}

/// `azurite` — the Azure Blob flavour of the `object_store` role, an
/// alternative to RustFS. Preferred in environments that need Azure-shaped storage. When
/// chosen, it provisions the demanded containers and vends the Azure connection string as an
/// object_store coordinate; RustFS is not deployed, so no `AWS_*` keys enter the stack.
fn azurite() -> Arc<dyn Module> {
    const CONN: &str = "DefaultEndpointsProtocol=http;AccountName=devstoreaccount1;AccountKey=Eby8vdM02xNOcqFlqUwJPLlmEtlCDXJ1OUzFT50uSRZ6IFsuFq2UVErCz4I6tq/K1SZFPTOtr/KBHBeksoGMGw==;BlobEndpoint=http://azurite:10000/devstoreaccount1;";
    let mut provides = Provides::default();
    // `AZURE_STORAGE_CONNECTION_STRING` is not hand-listed: the planner derives it from the
    // typed Azure credential below (via `Connection::standard_env`), so it is stated once and
    // enters `.env` only when Azurite is the chosen object_store. The blob port is the
    // emulator's fixed 10000, rendered directly in the fragment — no env var.

    // The Azure flavour of `object_store`: the same role-generic addressing as RustFS
    // (`uri`/`bucket`/`endpoint`), filled with the `wasbs://` shape, plus an Azure
    // connection-string credential.
    provides.resource_kinds.insert(
        Role::OBJECT_STORE.into(),
        ConnectionTemplate(Connection::ObjectStore {
            uri: "wasbs://{name}@devstoreaccount1.blob.core.windows.net".into(),
            bucket: "{name}".into(),
            // In-network direct address; the planner rewrites the origin to the gateway's
            // dedicated listener (keeping the `/devstoreaccount1` path) since `blob` is
            // `Gatewayed`. The `BlobEndpoint=` inside the connection string is rewritten too.
            endpoint: "http://azurite:10000/devstoreaccount1".into(),
            credential: ObjectStoreCredential::AzureBlob {
                connection_string: CONN.into(),
            },
            tls_trust: None,
        }),
    );
    // A consumer that demands `object_store` waits for the one-shot container init to finish.
    provides.ready_gate = Some(ReadyGate {
        service: "azurite-init".into(),
        readiness: Readiness::CompletedSuccessfully,
    });

    Arc::new(DataModule {
        id: ModuleId::from("azurite"),
        display_name: Some("Azurite".into()),
        summary: Some("Azure Blob emulator; object store with Azure-shaped wiring.".into()),
        category: Some("storage".into()),
        provider_of: Some(Role::OBJECT_STORE.into()),
        requires: vec![],
        // Same-role exclusivity is enforced by the planner (two object_store providers in one
        // environment is a `ConflictingRoleProviders` error), so no hand-listed
        // `conflicts_with` is needed here.
        conflicts_with: vec![],
        needs: vec![],
        service_specs: vec![ServiceSpec {
            name: "azurite".into(),
            role: Role::object_store(),
            placement: container("azurite"),
            // No raw host port: reached through the gateway's dedicated listener.
            endpoints: vec![Endpoint::gatewayed("blob", 10000)],
            depends_on: vec![],
            base_path: String::new(),
        }],
        provides,
        knobs: vec![
            image_knob("image", "Azurite image", images::AZURITE, "AZURITE_IMAGE"),
            image_knob(
                "init_image",
                "Azurite init image",
                images::AZURITE_INIT,
                "AZURITE_INIT_IMAGE",
            ),
        ],
        renders: template_with_files(
            include_str!("../../templates/modules/azurite/compose.yaml.jinja"),
            vec![sensitive_file(
                ".env/azurite-init.env",
                include_str!("../../templates/modules/azurite/init.env.jinja"),
            )],
        )
        .for_compose(),
    })
}

/// The default models [`mlflow`]'s AI Gateway is seeded with: `provider:model` pairs, each
/// becoming an endpoint named the model id (so clients asking for the model by its usual id
/// reach it unchanged).
pub const MLFLOW_AI_GATEWAY_MODELS_DEFAULT: &str =
    "anthropic:claude-opus-5-5,anthropic:claude-sonnet-5-5,anthropic:claude-haiku-5-5";

/// `mlflow` — experiment tracking and its built-in AI Gateway.
/// the Databricks-shaped tracking API, the OTel ingest path, the AI Gateway, and the UI. It
/// serves its whole surface under `/mlflow`, so the tracking API rewrites under that base, and
/// the Databricks-shaped OTel (`/api/2.0/otel`) and AI Gateway (`/ai-gateway`) mounts rewrite
/// onto the paths MLflow serves them at under it.
fn mlflow() -> Arc<dyn Module> {
    let mut provides = Provides::default();
    // `MLFLOW_S3_ENDPOINT_URL` and the AWS_* credentials are no longer declared here — they
    // are injected from the object_store provider's coordinates (see `needs` below), so they
    // follow whichever provider the planner chose and carry the in-network endpoint.
    //
    // The tracking API is Databricks-shaped (`/api/2.0/mlflow` at the gateway root), so a
    // client's tracking URI is the gateway origin; the MLflow client appends the API path.
    provides.client_env.insert(
        "MLFLOW_TRACKING_URI".into(),
        ClientValue::endpoint("mlflow", "tracking", UrlPart::Origin),
    );
    // The AI Gateway's base: a client appends the provider surface it speaks, e.g.
    // `/anthropic` (Claude Code's `ANTHROPIC_BASE_URL`) or `/openai/v1`.
    provides.client_env.insert(
        "MLFLOW_AI_GATEWAY_URL".into(),
        ClientValue::endpoint("mlflow", "ai_gateway", UrlPart::Full),
    );
    provides.python = PythonClient {
        dependencies: vec!["mlflow>=3".into(), "scikit-learn".into()],
        notebooks: vec![ClientNotebook {
            file: "mlflow_quickstart.py".into(),
            title: "train a scikit-learn model and track it in MLflow".into(),
            contents: include_str!("../../templates/modules/mlflow/notebooks/quickstart.py").into(),
        }],
        ..PythonClient::default()
    };

    Arc::new(Mlflow(DataModule {
        id: ModuleId::from("mlflow"),
        display_name: Some("MLflow".into()),
        summary: Some(
            "Experiment + model tracking and an AI Gateway; Databricks-shaped URLs.".into(),
        ),
        category: Some("ml".into()),
        provider_of: Some(Role::EXPERIMENT_TRACKING.into()),
        // Only the gateway is a hard module dependency; the relational store and
        // object store arrive via resource demands (auto-provisioned).
        requires: vec![ModuleId::from("envoy")],
        conflicts_with: vec![],
        // The relational store and object store arrive as demands. The role-generic
        // coordinates (`url`, `uri`, `endpoint`) are injected so the fragment no longer
        // hard-codes the backend URL, the `s3://` destination, or the endpoint. The S3
        // credentials themselves are *not* injected here — they are the chosen S3 provider's
        // own `env_vars` contribution (absent under an Azure provider), so MLflow's fragment
        // reads them with `:-` fallbacks.
        needs: vec![
            ResourceDemand {
                resource: Role::RELATIONAL_DB.into(),
                name: "mlflow".into(),
                provider: None,
                bind: ConnectionBinding {
                    bind: vec![(ConnectionField::Url, "MLFLOW_BACKEND_STORE_URI".into())],
                },
            },
            ResourceDemand {
                resource: Role::OBJECT_STORE.into(),
                name: "mlflow".into(),
                provider: None,
                bind: ConnectionBinding {
                    bind: vec![
                        (ConnectionField::Uri, "MLFLOW_ARTIFACTS_DESTINATION".into()),
                        (ConnectionField::Endpoint, "MLFLOW_S3_ENDPOINT_URL".into()),
                    ],
                },
            },
        ],
        service_specs: vec![ServiceSpec {
            name: "mlflow".into(),
            role: Role::experiment_tracking(),
            placement: container("mlflow"),
            // It serves its whole surface under `/mlflow`: the tracking API rewrites under that
            // base (`Inherit`), and the UI is fronted at the base path. OTLP ingest
            // (`/mlflow/v1/traces`) is mounted Databricks-style at `/api/2.0/otel`, and the AI
            // Gateway (`/mlflow/gateway/{anthropic,openai,mlflow}/…`) at `/ai-gateway`. The
            // gateway route streams model output and drops caller credentials, so MLflow
            // authenticates upstream with the provider key it stores rather than a client's
            // placeholder (it forwards a CLI's own key when one is present).
            base_path: "/mlflow".into(),
            endpoints: vec![
                Endpoint::api("tracking", 5000, "/api/2.0/mlflow", Rewrite::Inherit),
                Endpoint::api("otel", 5000, "/api/2.0/otel", Rewrite::To("/mlflow".into())),
                Endpoint::api(
                    "ai_gateway",
                    5000,
                    "/ai-gateway",
                    Rewrite::To("/mlflow/gateway".into()),
                )
                .streaming()
                .strip_client_credentials(),
                Endpoint::ui_prefixable("ui", 5000, None),
            ],
            // No hand-listed startup edges: MLflow's `depends_on` is demand-driven. The
            // planner injects the chosen relational-db / object-store providers' gates
            // (service + condition) into the fragment, so the wait follows whichever backend
            // it picked rather than naming `db`/`rustfs` here.
            depends_on: vec![],
        }],
        provides,
        knobs: vec![
            image_knob("image", "MLflow image", images::MLFLOW, "MLFLOW_IMAGE"),
            Knob {
                key: "ai_gateway_models".into(),
                title: Some("AI Gateway models".into()),
                kind: KnobKind::String,
                default: Some(MLFLOW_AI_GATEWAY_MODELS_DEFAULT.into()),
                required: false,
                help: Some(
                    "Comma-separated provider:model pairs to seed as AI Gateway endpoints, each \
                     named the model id. A pair is seeded only when its provider key (e.g. \
                     ANTHROPIC_API_KEY) is exported before `docker compose up`."
                        .into(),
                ),
                aliases: vec![],
                advanced: false,
            },
            Knob {
                key: "uc_url".into(),
                title: Some("Unity Catalog URL".into()),
                kind: KnobKind::String,
                default: Some(format!(
                    "http://{UNITY_CATALOG_SERVICE}:{UNITY_CATALOG_PORT}"
                )),
                required: false,
                help: Some(
                    "Where MLflow reaches Unity Catalog's REST API when both are in the \
                     environment, directly on the compose network: MLflow's model registry is \
                     then served from Unity Catalog."
                        .into(),
                ),
                aliases: vec![],
                advanced: true,
            },
        ],
        renders: template_with_files(
            include_str!("../../templates/modules/mlflow/compose.yaml.jinja"),
            vec![
                sensitive_file(
                    ".env/mlflow.env",
                    include_str!("../../templates/modules/mlflow/mlflow.env.jinja"),
                ),
                config_file(
                    "seed_gateway.py",
                    "mlflow_seed_gateway",
                    include_str!("../../templates/modules/mlflow/seed_gateway.py"),
                ),
                // A one-module Python distribution whose entry point registers MLflow's own
                // OSS Unity Catalog registry store for the `uc:` scheme, which `mlflow server`
                // otherwise only accepts client-side.
                config_file(
                    "uc-registry/trestle_mlflow_uc_registry.py",
                    "mlflow_uc_registry_module",
                    include_str!(
                        "../../templates/modules/mlflow/uc-registry/trestle_mlflow_uc_registry.py"
                    ),
                ),
                config_file(
                    "uc-registry/METADATA",
                    "mlflow_uc_registry_metadata",
                    include_str!("../../templates/modules/mlflow/uc-registry/METADATA"),
                ),
                config_file(
                    "uc-registry/entry_points.txt",
                    "mlflow_uc_registry_entry_points",
                    include_str!("../../templates/modules/mlflow/uc-registry/entry_points.txt"),
                ),
            ],
        )
        .for_compose(),
    }))
}

/// [`mlflow`]: a data module whose model registry moves to Unity Catalog when
/// `unity-catalog` is in the stack.
///
/// Both serve registered models, and an environment should have one registry. With UC present,
/// the server's registry store is UC (`--registry-store-uri uc:…`, see the fragment), so
/// MLflow's own registry API and UI read and write UC models, and clients get
/// `MLFLOW_REGISTRY_URI` to talk to UC's API directly. UC stays optional: MLflow alone keeps
/// its registry in its own database.
struct Mlflow(DataModule);

impl Module for Mlflow {
    fn id(&self) -> &ModuleId {
        self.0.id()
    }
    fn display_name(&self) -> Option<&str> {
        self.0.display_name()
    }
    fn summary(&self) -> Option<&str> {
        self.0.summary()
    }
    fn category(&self) -> Option<&str> {
        self.0.category()
    }
    fn provider_of(&self) -> Option<&str> {
        self.0.provider_of()
    }
    fn requires(&self) -> &[ModuleId] {
        self.0.requires()
    }
    fn conflicts_with(&self) -> &[ModuleId] {
        self.0.conflicts_with()
    }
    fn needs(&self) -> &[ResourceDemand] {
        self.0.needs()
    }
    fn knobs(&self) -> &[Knob] {
        self.0.knobs()
    }
    fn provides(&self) -> &Provides {
        self.0.provides()
    }
    fn render_for(&self, target: &TargetId) -> Option<&RenderSpec> {
        self.0.render_for(target)
    }
    fn services(&self, knobs: &ResolvedKnobs) -> Vec<ServiceSpec> {
        self.0.services(knobs)
    }
    fn client_env_in(&self, stack: &StackFacts) -> Cow<'_, BTreeMap<String, ClientValue>> {
        if !stack.has("unity-catalog") {
            return Cow::Borrowed(&self.0.provides.client_env);
        }
        // The MLflow client's OSS Unity Catalog registry: `uc:` + the origin it appends
        // `/api/2.1/unity-catalog` to (UC's REST mount sits at the gateway root).
        let mut env = self.0.provides.client_env.clone();
        env.insert(
            "MLFLOW_REGISTRY_URI".into(),
            ClientValue::module_endpoint(
                "unity-catalog",
                UNITY_CATALOG_SERVICE,
                "rest",
                UrlPart::Origin,
            )
            .with_prefix("uc:"),
        );
        Cow::Owned(env)
    }
}

/// Unity Catalog's compose service, and the port its REST API listens on in the container.
/// Shared with consumers that call UC directly on the compose network (see [`puppygraph`]).
const UNITY_CATALOG_SERVICE: &str = "unitycatalog";
const UNITY_CATALOG_PORT: u16 = 8080;

/// `unity-catalog` — the data catalog.
/// Databricks-shaped path at root, so `/api/2.1/unity-catalog` fronts with no rewrite;
/// a second `/unity-catalog` alias points at the same service.
fn unity_catalog() -> Arc<dyn Module> {
    let mut provides = Provides::default();
    provides.client_env.insert(
        "UNITY_CATALOG_URL".into(),
        ClientValue::endpoint(UNITY_CATALOG_SERVICE, "rest", UrlPart::Full),
    );
    // UC reads its storage config from a rendered `server.properties` and its metadata-store
    // config from a rendered `hibernate.properties` (both mounted as secrets: they carry keys
    // and a password). They read the typed connections directly, so this module injects no env
    // vars of its own. The S3 entries are what you'd write for
    // real AWS — no endpoint override, which UC doesn't support — so the store must answer the
    // AWS hostnames itself (see `rustfs`), and the fragment trusts the gateway's local CA when
    // the connection carries a `tls_trust`.

    Arc::new(DataModule {
        id: ModuleId::from("unity-catalog"),
        display_name: Some("Unity Catalog".into()),
        summary: Some("Open-source Unity Catalog server; Databricks-shaped REST API.".into()),
        category: Some("catalog".into()),
        provider_of: Some(Role::DATA_CATALOG.into()),
        // Only the gateway is a hard module dependency; Postgres + S3 arrive as demands.
        requires: vec![ModuleId::from("envoy")],
        conflicts_with: vec![],
        // The relational store and object store arrive as demands, but nothing is bound into
        // UC's env: its config files read the resolved connections directly —
        // `hibernate.properties` takes the database's typed parts (JDBC URL, user, password), so
        // the metadata lives in the provider rather than UC's embedded H2 file, and
        // `server.properties` takes the object stores.
        needs: vec![
            ResourceDemand {
                resource: Role::RELATIONAL_DB.into(),
                name: "unitycatalog".into(),
                provider: None,
                bind: ConnectionBinding::default(),
            },
            ResourceDemand {
                resource: Role::OBJECT_STORE.into(),
                name: "unity".into(),
                provider: None,
                bind: ConnectionBinding::default(),
            },
        ],
        service_specs: vec![ServiceSpec {
            name: UNITY_CATALOG_SERVICE.into(),
            role: Role::data_catalog(),
            placement: container(UNITY_CATALOG_SERVICE),
            // UC serves its REST API at root (empty base_path), so each mount `Inherit`s to no
            // rewrite: `/api/2.1/unity-catalog` fronts unchanged, and `/unity-catalog` is a
            // second alias to the same service.
            base_path: String::new(),
            endpoints: vec![
                Endpoint::api(
                    "rest",
                    UNITY_CATALOG_PORT,
                    "/api/2.1/unity-catalog",
                    Rewrite::Inherit,
                ),
                Endpoint::api(
                    "rest_alias",
                    UNITY_CATALOG_PORT,
                    "/unity-catalog",
                    Rewrite::Inherit,
                ),
            ],
            // Demand-driven, like MLflow: the planner injects the chosen providers' gates
            // (see the fragment's `depends_on`), so nothing is hand-listed here.
            depends_on: vec![],
        }],
        provides,
        knobs: vec![image_knob(
            "image",
            "Unity Catalog image",
            images::UNITY_CATALOG,
            "UC_IMAGE",
        )],
        renders: template_with_files(
            include_str!("../../templates/modules/unity-catalog/compose.yaml.jinja"),
            vec![
                secret_file(
                    "secrets/server.properties",
                    "unitycatalog_server_properties",
                    include_str!("../../templates/modules/unity-catalog/server.properties.jinja"),
                ),
                // The metadata store: the relational_db demand's connection, as the
                // Hibernate config the server reads (a secret: it holds the password).
                secret_file(
                    "secrets/hibernate.properties",
                    "unitycatalog_hibernate_properties",
                    include_str!(
                        "../../templates/modules/unity-catalog/hibernate.properties.jinja"
                    ),
                ),
            ],
        )
        .for_compose(),
    })
}

/// `jaeger` — all-in-one OTLP tracing backend, UI fronted at `/jaeger`.
fn jaeger() -> Arc<dyn Module> {
    let mut provides = Provides::default();
    // Clients export traces over OTLP/HTTP through the gateway (`/v1/traces` on the shared
    // listener), so host-side apps reach Jaeger without it publishing a port. The
    // signal-specific variable is used as-is by OTel SDKs; the generic
    // `OTEL_EXPORTER_OTLP_ENDPOINT` would also send metrics and logs, which Jaeger rejects.
    provides.client_env.insert(
        "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT".into(),
        ClientValue::endpoint("jaeger", "otlp_http", UrlPart::Full),
    );
    provides.client_env.insert(
        "OTEL_EXPORTER_OTLP_TRACES_PROTOCOL".into(),
        ClientValue::literal("http/protobuf"),
    );
    Arc::new(DataModule {
        id: ModuleId::from("jaeger"),
        display_name: Some("Jaeger".into()),
        summary: Some("All-in-one OTLP tracing backend with the Jaeger UI.".into()),
        category: Some("observability".into()),
        provider_of: Some(Role::TRACING.into()),
        requires: vec![ModuleId::from("envoy")],
        conflicts_with: vec![],
        needs: vec![],
        service_specs: vec![ServiceSpec {
            name: "jaeger".into(),
            role: Role::tracing(),
            placement: container("jaeger"),
            // The Jaeger UI is fronted at `/jaeger`.
            base_path: "/jaeger".into(),
            endpoints: vec![
                Endpoint::ui_prefixable("ui", 16686, Some(16686)),
                // OTLP/gRPC ingest, reached directly in-network (not gatewayed).
                Endpoint::internal("otlp_grpc", Scheme::Grpc, 4317, Some(4317)),
                // OTLP/HTTP trace ingest, fronted at the OTLP-standard path: the client-facing
                // export endpoint (see `client_env` above).
                Endpoint::api("otlp_http", 4318, "/v1/traces", Rewrite::Passthrough),
            ],
            depends_on: vec![],
        }],
        provides,
        knobs: vec![image_knob(
            "image",
            "Jaeger image",
            images::JAEGER,
            "JAEGER_IMAGE",
        )],
        renders: template(include_str!(
            "../../templates/modules/jaeger/compose.yaml.jinja"
        ))
        .for_compose(),
    })
}

/// The knob key gating Headwaters' bundled lineage UI.
const HEADWATERS_SERVE_UI: &str = "serve_ui";
const HEADWATERS_SERVE_UI_LEGACY: &str = "HEADWATERS_SERVE_UI";

/// Headwaters' baked-in, Databricks-style OpenLineage REST API path — a server constant the
/// planner must know to match the API route at the gateway (analogous to Unity Catalog's
/// `/api/2.1/unity-catalog`). The API is always served here; when the UI relocates the whole
/// surface under a base path, the gateway rewrites this prefix to carry the base path.
const HEADWATERS_API_PREFIX: &str = "/api/v1/lineage";

/// The base path Headwaters' whole surface (UI, REST read API, ConnectRPC, ingest) is served
/// under, told to the server via the generated `config.toml`'s `[ui].base_path`.
const HEADWATERS_BASE_PATH: &str = "/lineage";

/// `headwaters` — an OpenLineage lineage service, and the baseline's one *logic* module: the
/// set of services and routes it emits varies with a knob, so it implements [`Module`] by hand
/// rather than being a [`DataModule`].
///
/// Its whole surface is served under [`HEADWATERS_BASE_PATH`] (`/lineage`), and its OpenLineage
/// ingest path is additionally fronted at a fixed, Databricks-style path
/// ([`HEADWATERS_API_PREFIX`], `/api/v1/lineage`) rewritten under the base path. The bundled web
/// UI is gated on the `serve_ui` knob, which decides whether `/lineage` is fronted as a
/// [`UiPrefixable`](crate::RouteIntent::UiPrefixable) UI or as an [`Api`](crate::RouteIntent::Api) passthrough
/// (API-only, e.g. behind a custom UI) — the knob-driven service shape a `DataModule` cannot
/// express.
///
/// It demands only a `relational_db` (named `lineage`); Postgres provisions the database and
/// the planner injects the `db:service_healthy` gate.
struct Headwaters {
    id: ModuleId,
    needs: Vec<ResourceDemand>,
    knobs: Vec<Knob>,
    provides: Provides,
    render: RenderSpec,
    requires: Vec<ModuleId>,
}

impl Headwaters {
    fn new() -> Self {
        Headwaters {
            id: ModuleId::from("headwaters"),
            // Only the gateway is a hard module dependency; the relational store arrives as a
            // demand (auto-provisioned), and its startup gate is injected by the planner.
            requires: vec![ModuleId::from("envoy")],
            // The DSN is read straight from the resolved connection in the fragment
            // (`connections.relational_db.0.url`) — so nothing is bound into Headwaters' env
            // here.
            needs: vec![ResourceDemand {
                resource: Role::RELATIONAL_DB.into(),
                name: "lineage".into(),
                provider: None,
                bind: ConnectionBinding::default(),
            }],
            // The one user-tunable surface Headwaters exposes: whether to serve the bundled
            // lineage UI. The value lands in the module's `InjectedEnv` under
            // `HEADWATERS_SERVE_UI` (the generated `config.toml` reads it as `ui.serve`), and
            // it also drives `services()`'s route choice below.
            knobs: vec![
                Knob {
                    key: HEADWATERS_SERVE_UI.into(),
                    title: Some("Serve the lineage UI".into()),
                    kind: KnobKind::Bool,
                    default: Some("true".into()),
                    required: false,
                    help: Some(
                        "Serve the bundled lineage web UI. Turn off to run the service API-only \
                         (e.g. when embedding a custom UI built on the shipped components)."
                            .into(),
                    ),
                    aliases: vec![HEADWATERS_SERVE_UI_LEGACY.into()],
                    advanced: false,
                },
                image_knob(
                    "image",
                    "Headwaters image",
                    images::HEADWATERS,
                    "HEADWATERS_IMAGE",
                ),
            ],
            // The OpenLineage client contract (`OPENLINEAGE_URL` + `OPENLINEAGE_ENDPOINT`), as
            // read by e.g. `datafusion-openlineage`. The ingest path is the static API prefix
            // at the gateway regardless of the UI knob; the gateway applies any base-path
            // rewrite, so clients never see it.
            provides: Provides {
                client_env: [
                    (
                        "OPENLINEAGE_URL".to_string(),
                        ClientValue::endpoint("headwaters", "api", UrlPart::Origin),
                    ),
                    (
                        "OPENLINEAGE_ENDPOINT".to_string(),
                        ClientValue::endpoint("headwaters", "api", UrlPart::Path),
                    ),
                ]
                .into(),
                ..Provides::default()
            },
            // The service config is rendered to a mounted `config.toml` (so an environment's
            // effective Headwaters config is inspectable on disk) rather than threaded as
            // individual env vars; the DSN stays in `DATABASE_URL`, which Headwaters overlays
            // over the file so the secret never lands in the checked-in config.
            render: template_with_files(
                include_str!("../../templates/modules/headwaters/compose.yaml.jinja"),
                vec![
                    RenderFile {
                        path: "config.toml".into(),
                        contents: include_str!(
                            "../../templates/modules/headwaters/config.toml.jinja"
                        )
                        .into(),
                        alias: Some("headwaters_config".into()),
                        sensitive: false,
                        secret_alias: None,
                        at_root: false,
                        preserve: false,
                    },
                    sensitive_file(
                        ".env/headwaters-migrate.env",
                        include_str!("../../templates/modules/headwaters/database.env.jinja"),
                    ),
                    sensitive_file(
                        ".env/headwaters.env",
                        include_str!("../../templates/modules/headwaters/database.env.jinja"),
                    ),
                ],
            ),
        }
    }
}

impl Module for Headwaters {
    fn id(&self) -> &ModuleId {
        &self.id
    }
    fn display_name(&self) -> Option<&str> {
        Some("Headwaters")
    }
    fn summary(&self) -> Option<&str> {
        Some("OpenLineage lineage service; Databricks-style API + optional UI.")
    }
    fn category(&self) -> Option<&str> {
        Some("lineage")
    }
    fn provider_of(&self) -> Option<&str> {
        Some(Role::LINEAGE)
    }
    fn requires(&self) -> &[ModuleId] {
        &self.requires
    }
    fn needs(&self) -> &[ResourceDemand] {
        &self.needs
    }
    fn knobs(&self) -> &[Knob] {
        &self.knobs
    }
    fn provides(&self) -> &Provides {
        &self.provides
    }
    fn render_for(&self, target: &TargetId) -> Option<&RenderSpec> {
        (target.as_str() == TargetId::COMPOSE).then_some(&self.render)
    }
    fn services(&self, knobs: &ResolvedKnobs) -> Vec<ServiceSpec> {
        // The server always relocates its whole surface (UI, REST read API, ConnectRPC,
        // ingest) under `/lineage` (told via `config.toml`'s `[ui].base_path`), so the gateway
        // fronts that prefix either way:
        //
        // - UI on: as a `UiPrefixable` UI.
        // - UI off: as an `Api` passthrough — a custom UI built on the shipped components
        //   still needs the read API and ConnectRPC service behind it.
        //
        // The OpenLineage ingest path is also matched at its static, Databricks-style
        // `/api/v1/lineage` (like Unity Catalog's `/api/2.1/unity-catalog`) and rewritten to
        // carry the base path (`Rewrite::Inherit` → `/lineage/api/v1/lineage`), so clients
        // post to a stable path regardless of the knob.
        let serve_ui = knobs.bool(HEADWATERS_SERVE_UI, true);
        let surface = if serve_ui {
            Endpoint::ui_prefixable("ui", 8091, None)
        } else {
            // The same prefix, fronted as an API: REST reads, ConnectRPC and ingest, but no UI.
            Endpoint::api("surface", 8091, HEADWATERS_BASE_PATH, Rewrite::Passthrough)
        };
        let endpoints = vec![
            surface,
            Endpoint::api("api", 8091, HEADWATERS_API_PREFIX, Rewrite::Inherit),
        ];
        vec![ServiceSpec {
            name: "headwaters".into(),
            role: Role::lineage(),
            placement: container("headwaters"),
            base_path: HEADWATERS_BASE_PATH.into(),
            endpoints,
            // Demand-driven, like MLflow: the planner injects the chosen relational-db
            // provider's gate into the fragment's `depends_on`.
            depends_on: vec![],
        }]
    }
}

fn headwaters() -> Arc<dyn Module> {
    Arc::new(Headwaters::new())
}

/// The [`omnigent`] knob for running an agent host inside the stack.
const OMNIGENT_HOST_KNOB: &str = "host";
/// Where Omnigent serves its whole surface (web UI, `/v1` API, WebSockets, SSE) at the gateway.
const OMNIGENT_BASE_PATH: &str = "/omnigent";
/// The port Omnigent's server image listens on.
const OMNIGENT_PORT: u16 = 8000;

/// `omnigent` — the Omnigent agent platform: a coordinating server (web UI + API) behind the
/// gateway, and an agent host in the stack that runs the agents.
///
/// The host is what makes agents part of the environment. It loads the stack's
/// container-vantage client env (every selected module's address, through the gateway) and
/// forwards it to each agent. It gets its models from [`mlflow`]'s AI Gateway, which holds
/// the provider keys, so the host carries none.
///
/// The host runs only while the gateway is open. Under forward-auth the server trusts the
/// gateway-asserted `Remote-Email`, and a host can't present that identity, so it is dropped;
/// browsers and laptop hosts still reach the server through the gateway.
struct Omnigent {
    id: ModuleId,
    requires: Vec<ModuleId>,
    needs: Vec<ResourceDemand>,
    knobs: Vec<Knob>,
    provides: Provides,
    render: RenderSpec,
}

impl Omnigent {
    fn new() -> Self {
        let string_knob = |key: &str, title: &str, default: &str, help: &str| Knob {
            key: key.into(),
            title: Some(title.into()),
            kind: KnobKind::String,
            default: Some(default.into()),
            required: false,
            help: Some(help.into()),
            aliases: vec![],
            advanced: false,
        };
        Omnigent {
            id: ModuleId::from("omnigent"),
            // The gateway fronts the server; MLflow's AI Gateway serves the agents' models (and
            // brings its own relational store and object store through its demands).
            requires: vec![ModuleId::from("envoy"), ModuleId::from("mlflow")],
            // The server's state (sessions, agents, hosts) lives in the stack's relational store;
            // the fragment reads the DSN straight from the resolved connection.
            needs: vec![ResourceDemand {
                resource: Role::RELATIONAL_DB.into(),
                name: "omnigent".into(),
                provider: None,
                bind: ConnectionBinding::default(),
            }],
            knobs: vec![
                Knob {
                    key: OMNIGENT_HOST_KNOB.into(),
                    title: Some("Run an agent host in the stack".into()),
                    kind: KnobKind::Bool,
                    default: Some("true".into()),
                    required: false,
                    help: Some(
                        "Run agents in a host container on the stack's network, with the \
                         lakehouse's client env and models from MLflow's AI Gateway. Off: \
                         attach your own host with `omnigent host $OMNIGENT_URL`. Always off \
                         while the gateway authenticates."
                            .into(),
                    ),
                    aliases: vec![],
                    advanced: false,
                },
                string_knob(
                    "model",
                    "Agent model",
                    "claude-opus-5-5",
                    "The model agents use by default: an AI Gateway endpoint name (see \
                     mlflow.ai_gateway_models).",
                ),
                string_knob(
                    "fast_model",
                    "Background model",
                    "claude-haiku-5-5",
                    "The model harnesses use for quick background calls: an AI Gateway \
                     endpoint name.",
                ),
                image_knob(
                    "image",
                    "Omnigent server image",
                    images::OMNIGENT_SERVER,
                    "OMNIGENT_IMAGE",
                ),
                image_knob(
                    "host_image",
                    "Omnigent host image",
                    images::OMNIGENT_HOST,
                    "OMNIGENT_HOST_IMAGE",
                ),
            ],
            // Where a client (`omnigent run --server`, `omnigent host`) reaches the server: its
            // whole surface sits under the base path.
            provides: Provides {
                client_env: [(
                    "OMNIGENT_URL".to_string(),
                    ClientValue::endpoint("omnigent", "ui", UrlPart::Full),
                )]
                .into(),
                ..Provides::default()
            },
            render: template_with_files(
                include_str!("../../templates/modules/omnigent/compose.yaml.jinja"),
                vec![sensitive_file(
                    ".env/omnigent.env",
                    include_str!("../../templates/modules/omnigent/omnigent.env.jinja"),
                )],
            ),
        }
    }
}

impl Module for Omnigent {
    fn id(&self) -> &ModuleId {
        &self.id
    }
    fn display_name(&self) -> Option<&str> {
        Some("Omnigent")
    }
    fn summary(&self) -> Option<&str> {
        Some("AI-agent platform; agents run in the stack, models via MLflow's AI Gateway.")
    }
    fn category(&self) -> Option<&str> {
        Some("agents")
    }
    fn provider_of(&self) -> Option<&str> {
        Some(Role::AGENT_PLATFORM)
    }
    fn requires(&self) -> &[ModuleId] {
        &self.requires
    }
    fn needs(&self) -> &[ResourceDemand] {
        &self.needs
    }
    fn knobs(&self) -> &[Knob] {
        &self.knobs
    }
    fn provides(&self) -> &Provides {
        &self.provides
    }
    fn render_for(&self, target: &TargetId) -> Option<&RenderSpec> {
        (target.as_str() == TargetId::COMPOSE).then_some(&self.render)
    }
    fn services(&self, knobs: &ResolvedKnobs) -> Vec<ServiceSpec> {
        self.services_in(knobs, &StackFacts::default())
    }
    fn services_in(&self, knobs: &ResolvedKnobs, stack: &StackFacts) -> Vec<ServiceSpec> {
        // The server relocates its whole surface under the base path (told via
        // `OMNIGENT_WEB_BASE_PATH`), so one prefixable endpoint fronts the UI, the `/v1` API and
        // its WebSockets. Session views stream over SSE, so the route carries no timeout.
        let mut services = vec![ServiceSpec {
            name: "omnigent".into(),
            role: Role::agent_platform(),
            placement: container("omnigent"),
            base_path: OMNIGENT_BASE_PATH.into(),
            endpoints: vec![Endpoint::ui_prefixable("ui", OMNIGENT_PORT, None).streaming()],
            // Demand-driven: the planner injects the chosen relational-db provider's gate.
            depends_on: vec![],
        }];
        if knobs.bool(OMNIGENT_HOST_KNOB, true) && !stack.gateway_auth {
            // The host dials the server directly on the compose network (it serves nothing).
            services.push(ServiceSpec {
                name: "omnigent-host".into(),
                role: Role::agent_platform(),
                placement: container("omnigent-host"),
                base_path: String::new(),
                endpoints: vec![],
                depends_on: vec!["omnigent".into()],
            });
        }
        services
    }
}

fn omnigent() -> Arc<dyn Module> {
    Arc::new(Omnigent::new())
}

/// Where marimo serves its editor (pages, API, kernel WebSockets) at the gateway.
const MARIMO_BASE_PATH: &str = "/marimo";
/// The port the marimo editor listens on in its container.
const MARIMO_PORT: u16 = 2718;

/// `marimo` — the stack's Python client workspace served by a hosted
/// [marimo](https://marimo.io) editor, behind the gateway at `/marimo`.
///
/// It edits the same `client/` workspace as the host workflow (`uv run marimo edit
/// notebooks`), so selecting it generates the workspace
/// ([`requires_workspace`](crate::PythonClient::requires_workspace)). Inside the container the
/// notebooks load the in-network `container.env` in place of `host.env`, and packages install
/// into a container-owned virtualenv, so the host's `client/.venv` is never touched.
///
/// Single-user: the editor runs without its own token, so the gateway (and its sign-in, when
/// enabled) is the only boundary. Anyone who reaches `/marimo` can run code in the stack.
fn marimo() -> Arc<dyn Module> {
    let mut provides = Provides::default();
    provides.client_env.insert(
        "MARIMO_URL".into(),
        ClientValue::endpoint("marimo", "ui", UrlPart::Full),
    );
    provides.python.requires_workspace = true;
    Arc::new(DataModule {
        id: ModuleId::from("marimo"),
        display_name: Some("marimo".into()),
        summary: Some("Hosted marimo editor for the client workspace's notebooks.".into()),
        category: Some("notebooks".into()),
        provider_of: Some(Role::NOTEBOOKS.into()),
        requires: vec![ModuleId::from("envoy")],
        conflicts_with: vec![],
        needs: vec![],
        service_specs: vec![ServiceSpec {
            name: "marimo".into(),
            role: Role::notebooks(),
            placement: container("marimo"),
            // `--base-url` moves the whole editor under the prefix; kernels talk over a
            // long-lived WebSocket, so the route carries no timeout.
            base_path: MARIMO_BASE_PATH.into(),
            endpoints: vec![Endpoint::ui_prefixable("ui", MARIMO_PORT, None).streaming()],
            depends_on: vec![],
        }],
        provides,
        knobs: vec![image_knob(
            "image",
            "marimo runtime image",
            images::MARIMO,
            "MARIMO_IMAGE",
        )],
        renders: template(include_str!(
            "../../templates/modules/marimo/compose.yaml.jinja"
        ))
        .for_compose(),
    })
}

/// The ports PuppyGraph listens on in its container: the web UI + REST API, Gremlin (over
/// WebSocket), and openCypher over Bolt.
const PUPPYGRAPH_UI_PORT: u16 = 8081;
const PUPPYGRAPH_GREMLIN_PORT: u16 = 8182;
const PUPPYGRAPH_BOLT_PORT: u16 = 7687;

/// `puppygraph` — [PuppyGraph](https://docs.puppygraph.com), a graph query engine (openCypher
/// over Bolt, Gremlin) that reads the lakehouse's Delta tables in place, with Unity Catalog as
/// its metastore.
///
/// The graph's shape (which tables are nodes and edges) is operator-owned: `graphs/puppygraph/
/// schema.json` is written beside `compose.yaml` once and never overwritten. The connection
/// half is generated: a `catalog.json` naming the UC metastore. On every start a one-shot
/// (`puppygraph-schema`) merges the two and uploads the result to PuppyGraph, unless the schema
/// maps no nodes yet (so a schema built in the web UI isn't replaced by an empty one).
///
/// UC is called directly on the compose network, not through the gateway: with forward-auth on,
/// an in-network metastore call through the gateway would be rejected. PuppyGraph reads the
/// Delta files with credentials UC vends, at the real AWS S3 hostnames, which the object store
/// answers through the gateway's local CA (see `rustfs`). The `object_store` demand (named like
/// UC's) carries that CA's [`TlsTrust`](crate::TlsTrust) and the store's startup gate; its keys
/// are not used. UC vends no credentials for an Azure store, so on Azurite the graph can't read
/// table data.
///
/// The web UI only serves at `/`, so it gets a dedicated gateway listener; Bolt and Gremlin are
/// published on their standard ports. All three use PuppyGraph's own login (the `password`
/// knob); the dedicated listener is not behind the gateway's sign-in.
fn puppygraph() -> Arc<dyn Module> {
    let mut provides = Provides::default();
    for (key, endpoint, part) in [
        ("PUPPYGRAPH_URL", "ui", UrlPart::Full),
        ("PUPPYGRAPH_BOLT_ADDRESS", "bolt", UrlPart::Authority),
        ("PUPPYGRAPH_GREMLIN_ADDRESS", "gremlin", UrlPart::Authority),
    ] {
        provides.client_env.insert(
            key.into(),
            ClientValue::endpoint("puppygraph", endpoint, part),
        );
    }
    provides.python = PythonClient {
        dependencies: vec!["neo4j>=5".into(), "gremlinpython>=3.7".into()],
        notebooks: vec![ClientNotebook {
            file: "puppygraph_quickstart.py".into(),
            title: "query the lakehouse as a graph with PuppyGraph (Cypher and Gremlin)".into(),
            contents: include_str!("../../templates/modules/puppygraph/notebooks/quickstart.py")
                .into(),
        }],
        ..PythonClient::default()
    };
    let string_knob = |key: &str, title: &str, default: &str, help: &str, advanced: bool| Knob {
        key: key.into(),
        title: Some(title.into()),
        kind: KnobKind::String,
        default: Some(default.into()),
        required: false,
        help: Some(help.into()),
        aliases: vec![],
        advanced,
    };
    let uc_url = format!("http://{UNITY_CATALOG_SERVICE}:{UNITY_CATALOG_PORT}");
    Arc::new(DataModule {
        id: ModuleId::from("puppygraph"),
        display_name: Some("PuppyGraph".into()),
        summary: Some("Graph queries (Cypher, Gremlin) over Unity Catalog tables.".into()),
        category: Some("graph".into()),
        provider_of: Some(Role::GRAPH.into()),
        requires: vec![ModuleId::from("envoy"), ModuleId::from("unity-catalog")],
        conflicts_with: vec![],
        // Named like UC's demand, so it resolves to the store UC vends credentials for. Nothing
        // is bound into env: the fragment reads the connection's CA trust directly.
        needs: vec![ResourceDemand {
            resource: Role::OBJECT_STORE.into(),
            name: "unity".into(),
            provider: None,
            bind: ConnectionBinding::default(),
        }],
        service_specs: vec![ServiceSpec {
            name: "puppygraph".into(),
            role: Role::graph(),
            placement: container("puppygraph"),
            base_path: String::new(),
            endpoints: vec![
                Endpoint::ui_fixed("ui", PUPPYGRAPH_UI_PORT),
                Endpoint::internal(
                    "gremlin",
                    Scheme::Http,
                    PUPPYGRAPH_GREMLIN_PORT,
                    Some(PUPPYGRAPH_GREMLIN_PORT),
                ),
                Endpoint::internal(
                    "bolt",
                    Scheme::Tcp,
                    PUPPYGRAPH_BOLT_PORT,
                    Some(PUPPYGRAPH_BOLT_PORT),
                ),
            ],
            // Demand-driven: the planner injects the object store's gate.
            depends_on: vec![],
        }],
        provides,
        knobs: vec![
            string_knob(
                "catalog",
                "Unity Catalog catalog",
                "unity",
                "The UC catalog whose schemas and tables the graph schema maps.",
                false,
            ),
            string_knob(
                "uc_url",
                "Unity Catalog URL",
                &uc_url,
                "Where PuppyGraph reaches Unity Catalog's REST API, directly on the compose \
                 network (not through the gateway).",
                true,
            ),
            string_knob(
                "password",
                "PuppyGraph password",
                "puppygraph123",
                "Password of the `puppygraph` user for the web UI, Bolt and Gremlin. A local \
                 default; set your own if the ports are reachable by others.",
                true,
            ),
            image_knob(
                "image",
                "PuppyGraph image",
                images::PUPPYGRAPH,
                "PUPPYGRAPH_IMAGE",
            ),
        ],
        renders: template_with_files(
            include_str!("../../templates/modules/puppygraph/compose.yaml.jinja"),
            vec![
                sensitive_file(
                    ".env/puppygraph.env",
                    include_str!("../../templates/modules/puppygraph/puppygraph.env.jinja"),
                ),
                // The connection half of the schema (the UC metastore), regenerated on every
                // render so knob changes flow through.
                config_file(
                    "catalog.json",
                    "puppygraph_catalog",
                    include_str!("../../templates/modules/puppygraph/catalog.json.jinja"),
                ),
                config_file(
                    "seed_schema.py",
                    "puppygraph_seed_schema",
                    include_str!("../../templates/modules/puppygraph/seed_schema.py"),
                ),
                operator_file(
                    "graphs/puppygraph/schema.json",
                    "puppygraph_schema",
                    include_str!("../../templates/modules/puppygraph/schema.json"),
                ),
            ],
        )
        .for_compose(),
    })
}

/// `opa` — Open Policy Agent as the gateway's authorization engine.
///
/// Selecting it adds a gRPC `ext_authz` check to the gateway's shared listener, *after*
/// forward-auth, so the policy decides on the identity the auth provider asserted (the
/// `Remote-User` / `Remote-Groups` headers). Selecting it turns gateway auth on; with auth
/// explicitly off the plan is a [`PlanError::PolicyRequiresAuth`](crate::PlanError).
///
/// The policy is operator-owned: `policies/authz.rego` (plus its tests) is written beside
/// `compose.yaml` once and never overwritten.
fn opa() -> Arc<dyn Module> {
    Arc::new(DataModule {
        id: ModuleId::from("opa"),
        display_name: Some("Open Policy Agent".into()),
        summary: Some("Rego authorization for every gateway request, after sign-on.".into()),
        category: Some("gateway".into()),
        provider_of: Some(Role::POLICY.into()),
        // Like Authelia, located by role: Envoy depends on it at runtime, not the reverse.
        requires: vec![],
        conflicts_with: vec![],
        needs: vec![],
        service_specs: vec![ServiceSpec {
            name: "opa".into(),
            role: Role::policy(),
            placement: container("opa"),
            // The ext_authz gRPC port; the planner builds the gateway's HTTP/2 cluster from it.
            endpoints: vec![Endpoint::internal("grpc", Scheme::Grpc, 9191, None)],
            depends_on: vec![],
            base_path: String::new(),
        }],
        provides: Provides::default(),
        knobs: vec![
            image_knob("image", "OPA image", images::OPA, "OPA_IMAGE"),
            Knob {
                key: "dry_run".into(),
                title: Some("Dry run".into()),
                kind: KnobKind::Bool,
                default: Some("false".into()),
                required: false,
                help: Some(
                    "Allow every request and only log the policy's decision — for rolling out \
                     a policy change safely."
                        .into(),
                ),
                aliases: vec![],
                advanced: true,
            },
            Knob {
                key: "log_level".into(),
                title: Some("Log level".into()),
                kind: KnobKind::Enum {
                    options: ["debug", "info", "warn", "error"]
                        .map(String::from)
                        .to_vec(),
                },
                default: Some("info".into()),
                required: false,
                help: None,
                aliases: vec![],
                advanced: true,
            },
        ],
        renders: template_with_files(
            include_str!("../../templates/modules/opa/compose.yaml.jinja"),
            vec![
                config_file(
                    "config.yaml",
                    "opa_config",
                    include_str!("../../templates/modules/opa/config.yaml.jinja"),
                ),
                operator_file(
                    "policies/authz.rego",
                    "opa_policy",
                    include_str!("../../templates/modules/opa/authz.rego"),
                ),
                operator_file(
                    "policies/authz_test.rego",
                    "opa_policy_test",
                    include_str!("../../templates/modules/opa/authz_test.rego"),
                ),
            ],
        )
        .for_compose(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::endpoint::RouteIntent;

    #[test]
    fn headwaters_emits_api_and_optional_ui_per_knob() {
        let m = headwaters();
        fn ep<'a>(s: &'a ServiceSpec, id: &str) -> &'a Endpoint {
            s.endpoints
                .iter()
                .find(|e| e.id == id)
                .unwrap_or_else(|| panic!("no endpoint {id}"))
        }

        // UI on (the default): two endpoints on one service — the UI prefixable at `/lineage`,
        // and the API matched at its static path but rewritten to carry the base path.
        let mut on = ResolvedKnobs::new();
        on.set(HEADWATERS_SERVE_UI, "true");
        let svcs = m.services(&on);
        assert_eq!(svcs.len(), 1);
        assert_eq!(svcs[0].base_path, "/lineage");
        assert_eq!(ep(&svcs[0], "ui").intent, RouteIntent::UiPrefixable);
        assert_eq!(ep(&svcs[0], "ui").mount_prefix, None);
        let api = ep(&svcs[0], "api");
        assert_eq!(api.intent, RouteIntent::Api);
        assert_eq!(api.mount_prefix.as_deref(), Some(HEADWATERS_API_PREFIX));
        // `Inherit` against base_path `/lineage` joins to the relocated upstream path.
        assert_eq!(api.rewrite, Rewrite::Inherit);

        // UI off: the surface stays under the base path, fronted as an API passthrough (the
        // read API and ConnectRPC remain reachable), and ingest keeps its static path.
        let mut off = ResolvedKnobs::new();
        off.set(HEADWATERS_SERVE_UI, "false");
        let svcs = m.services(&off);
        assert_eq!(svcs[0].base_path, HEADWATERS_BASE_PATH);
        assert!(svcs[0].endpoints.iter().all(|e| e.id != "ui"));
        let surface = ep(&svcs[0], "surface");
        assert_eq!(surface.intent, RouteIntent::Api);
        assert_eq!(surface.mount_prefix.as_deref(), Some(HEADWATERS_BASE_PATH));
        assert_eq!(surface.rewrite, Rewrite::Passthrough);
        let api = ep(&svcs[0], "api");
        assert_eq!(api.intent, RouteIntent::Api);
        assert_eq!(api.mount_prefix.as_deref(), Some(HEADWATERS_API_PREFIX));
        assert_eq!(api.rewrite, Rewrite::Inherit);

        // The default (no knob value resolved) keeps the UI on.
        let svcs = m.services(&ResolvedKnobs::new());
        assert!(svcs[0].endpoints.iter().any(|e| e.id == "ui"));
    }
}
