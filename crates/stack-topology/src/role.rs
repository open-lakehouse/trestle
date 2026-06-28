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
}

impl ServiceSpec {
    /// Look up one of this service's endpoints by id.
    pub fn endpoint(&self, id: &str) -> Option<&Endpoint> {
        self.endpoints.iter().find(|e| e.id == id)
    }
}
