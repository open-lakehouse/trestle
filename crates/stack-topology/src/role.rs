//! What a service *is* in the architecture ([`Role`]), kept separate from *which*
//! implementation fills it ([`ServiceSpec::name`]).
//!
//! This separation is the crate's forward-looking invariant: the framework names
//! **roles** (`data_catalog`, `object_store`, `gateway`, …), never
//! implementations. "Unity Catalog" is a *value* — a [`ServiceSpec`] with
//! `name: "unity-catalog"` and `role: Role::data_catalog()` — not a type. A second
//! `data_catalog` (an Iceberg REST Catalog) or a hybrid (both present) drops into a
//! catalog without touching any type here.

use serde::{Deserialize, Serialize};

use crate::endpoint::Endpoint;
use crate::placement::Placement;

/// The role a service fills in the architecture.
///
/// An **open set** — a string newtype, not an enum — so a new role needs no change
/// to this crate. Conventionally lower-snake-case (`"data_catalog"`,
/// `"object_store"`, `"gateway"`, `"sql_engine"`, `"lineage"`,
/// `"experiment_tracking"`, `"tracing"`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Role(pub String);

impl Role {
    /// Construct a role from anything string-like.
    pub fn new(s: impl Into<String>) -> Self {
        Role(s.into())
    }

    /// The role identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The well-known roles, as `&'static str` constants and matching constructors.
///
/// These are *discoverability and one-spelling-source* helpers over the open set —
/// they do **not** close it. A catalog may still use any role string; these just spare
/// the common ones from stringly-typed typos and make them greppable.
impl Role {
    /// `object_store` — an S3/Blob-style object store.
    pub const OBJECT_STORE: &'static str = "object_store";
    /// `relational_db` — a relational database (provisions named databases).
    pub const RELATIONAL_DB: &'static str = "relational_db";
    /// `data_catalog` — a metadata/data catalog (e.g. a Unity Catalog server).
    pub const DATA_CATALOG: &'static str = "data_catalog";
    /// `gateway` — the single-port front-edge gateway.
    pub const GATEWAY: &'static str = "gateway";
    /// `sql_engine` — a distributed SQL query engine.
    pub const SQL_ENGINE: &'static str = "sql_engine";
    /// `experiment_tracking` — an ML experiment/model tracking server.
    pub const EXPERIMENT_TRACKING: &'static str = "experiment_tracking";
    /// `tracing` — an OTLP tracing backend.
    pub const TRACING: &'static str = "tracing";
    /// `lineage` — a data-lineage service (e.g. an OpenLineage backend).
    pub const LINEAGE: &'static str = "lineage";
    /// `app_runtime` — the application-runtime contract (env-only).
    pub const APP_RUNTIME: &'static str = "app_runtime";
    /// `auth` — a forward-auth / single-sign-on provider for the gateway.
    pub const AUTH: &'static str = "auth";

    /// The `object_store` role.
    pub fn object_store() -> Role {
        Role::new(Self::OBJECT_STORE)
    }
    /// The `relational_db` role.
    pub fn relational_db() -> Role {
        Role::new(Self::RELATIONAL_DB)
    }
    /// The `data_catalog` role.
    pub fn data_catalog() -> Role {
        Role::new(Self::DATA_CATALOG)
    }
    /// The `gateway` role.
    pub fn gateway() -> Role {
        Role::new(Self::GATEWAY)
    }
    /// The `sql_engine` role.
    pub fn sql_engine() -> Role {
        Role::new(Self::SQL_ENGINE)
    }
    /// The `experiment_tracking` role.
    pub fn experiment_tracking() -> Role {
        Role::new(Self::EXPERIMENT_TRACKING)
    }
    /// The `tracing` role.
    pub fn tracing() -> Role {
        Role::new(Self::TRACING)
    }
    /// The `lineage` role.
    pub fn lineage() -> Role {
        Role::new(Self::LINEAGE)
    }
    /// The `app_runtime` role.
    pub fn app_runtime() -> Role {
        Role::new(Self::APP_RUNTIME)
    }
    /// The `auth` role.
    pub fn auth() -> Role {
        Role::new(Self::AUTH)
    }
}

impl From<&str> for Role {
    fn from(s: &str) -> Self {
        Role(s.to_string())
    }
}

impl From<String> for Role {
    fn from(s: String) -> Self {
        Role(s)
    }
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// One service in an environment: a concrete implementation of a [`Role`], where
/// it runs, what it offers, and what it depends on.
///
/// This is the unit the resolvers operate over. Selection (which specs are in an
/// environment) and the placement assignment (host vs container) live in the
/// consuming tool; this type only describes a spec once chosen.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceSpec {
    /// The implementation identifier (e.g. `"unity-catalog"`, `"iceberg-rest"`,
    /// `"seaweedfs"`, `"azurite"`, `"marquez"`). Unique within an environment.
    pub name: String,
    /// What this service *is* in the architecture, independent of `name`.
    pub role: Role,
    /// Where this service runs in this deployment shape.
    pub placement: Placement,
    /// The endpoints this service offers.
    #[serde(default)]
    pub endpoints: Vec<Endpoint>,
    /// Other services (by `name`) this one directly depends on.
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Where this service serves itself, as a root-relative path (e.g. `"/mlflow"`,
    /// `"/lineage"`) — the typed replacement for the old `base_path` extra. Empty means it
    /// serves at root. The planner uses this as a [`UiPrefixable`](crate::RouteIntent::UiPrefixable)
    /// endpoint's chosen base path and as the base an [`Api`](crate::RouteIntent::Api)
    /// endpoint's [`Rewrite::Inherit`](crate::Rewrite::Inherit) joins its mount prefix onto.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub base_path: String,
}

impl ServiceSpec {
    /// Look up one of this service's endpoints by id.
    pub fn endpoint(&self, id: &str) -> Option<&Endpoint> {
        self.endpoints.iter().find(|e| e.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_ctor_and_const_agree() {
        // The ctor and the const name the same open-set role string.
        assert_eq!(Role::object_store().as_str(), Role::OBJECT_STORE);
        // A custom role is a perfectly valid Role outside the well-known set.
        assert_eq!(Role::new("quantum_db").as_str(), "quantum_db");
    }
}
