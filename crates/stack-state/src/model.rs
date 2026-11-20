//! The records the store holds.

use std::collections::BTreeMap;
use std::fmt;
use std::time::SystemTime;

use olai_stack_topology::EnvManifest;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

macro_rules! id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            pub(crate) fn generate() -> Self {
                $name(uuid::Uuid::new_v4().to_string())
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id!(
    /// A [`Workspace`]'s id.
    WorkspaceId
);
id!(
    /// An [`Environment`]'s id.
    EnvironmentId
);
id!(
    /// An [`Instance`]'s id.
    InstanceId
);
id!(
    /// An [`Attachment`]'s id.
    AttachmentId
);

/// Where a shared store's database is. It names the database, never a credential: whoever
/// opens it resolves a connection string with their own login
/// ([`Store::open_attached`](crate::Store::open_attached)).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteLocation {
    /// A database on a Neon project's branch, reached with the user's Neon CLI login.
    Neon {
        /// The organization, when the user belongs to more than one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        org: Option<String>,
        /// The project id.
        project: String,
        /// The branch (its name or id); the project's default branch when `None`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        branch: Option<String>,
        /// The database name.
        database: String,
    },
}

impl fmt::Display for RemoteLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RemoteLocation::Neon {
                org,
                project,
                branch,
                database,
            } => {
                write!(f, "neon:")?;
                if let Some(org) = org {
                    write!(f, "{org}/")?;
                }
                write!(f, "{project}")?;
                if let Some(branch) = branch {
                    write!(f, "@{branch}")?;
                }
                write!(f, "/{database}")
            }
        }
    }
}

/// A shared store attached on this machine: a team database whose workspaces and
/// environments this machine runs instances of. The record lives in the local `state.db`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attachment {
    /// Its id, which this machine's instances of the store's revisions record.
    pub id: AttachmentId,
    /// What the user calls it on this machine (unique here).
    pub name: String,
    /// Where the database is.
    pub location: RemoteLocation,
}

/// Where a secret's value lives. The store keeps only this reference; the value stays with
/// the provider (the OS keychain, the process environment).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SecretRef {
    /// The provider that resolves it (e.g. `env`, `keychain`).
    pub provider: String,
    /// The key the provider knows the value by.
    pub key: String,
}

/// One variable's value: a literal, or a reference to a secret.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VarValue {
    /// The value itself.
    Literal(String),
    /// Where the value lives.
    Secret(SecretRef),
}

impl fmt::Debug for VarValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // A literal may still be something the user would rather not see in a log.
            VarValue::Literal(_) => f.write_str("Literal(<redacted>)"),
            VarValue::Secret(r) => f.debug_tuple("Secret").field(r).finish(),
        }
    }
}

/// Named variables, as a workspace, an environment, a revision or an instance sets them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct VarSet(pub BTreeMap<String, VarValue>);

impl VarSet {
    /// No variables.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set `name` to `value`.
    pub fn set(&mut self, name: impl Into<String>, value: VarValue) -> &mut Self {
        self.0.insert(name.into(), value);
        self
    }

    /// The value of `name`.
    pub fn get(&self, name: &str) -> Option<&VarValue> {
        self.0.get(name)
    }

    /// Remove `name`, returning its value if it was set.
    pub fn remove(&mut self, name: &str) -> Option<VarValue> {
        self.0.remove(name)
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Which mutable [`VarSet`] a change applies to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VarScope {
    /// The vars every environment of the workspace sees.
    Workspace(WorkspaceId),
    /// The vars every revision of the environment sees.
    Environment(EnvironmentId),
    /// The vars the environment's next revision carries.
    Draft(EnvironmentId),
    /// This machine's own vars for the environment, on top of every shared layer and never
    /// shared: the minted keys sealing its instances' data, the user's own API keys. Kept in
    /// the local `state.db` even when the environment is held in an attached store.
    Local(EnvironmentId),
}

/// An overlay source an instance takes: its name, and what the source needs to know about the
/// instance (which branch, say). The config is merged over the workspace's config for the same
/// source. Neither holds secrets.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceBinding {
    /// The source's name (e.g. `user`).
    pub name: String,
    /// What the source is told about this instance; `null` for nothing.
    #[serde(default)]
    pub config: serde_json::Value,
}

impl SourceBinding {
    /// A binding to `name` with no config.
    pub fn new(name: impl Into<String>) -> Self {
        SourceBinding {
            name: name.into(),
            config: serde_json::Value::Null,
        }
    }

    /// The same binding with `config`.
    pub fn with_config(mut self, config: serde_json::Value) -> Self {
        self.config = config;
        self
    }
}

/// A set of environments sharing a Docker host, their vars, and whether their gateways share a
/// mesh network.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Workspace {
    /// Its id.
    pub id: WorkspaceId,
    /// Its name: the prefix of its instances' compose projects.
    pub name: String,
    /// Whether its environments' gateways join a shared network.
    pub mesh: bool,
    /// Vars every environment in the workspace sees.
    pub vars: VarSet,
    /// Per overlay source, the config every instance of the workspace binds it with (e.g. which
    /// project), below the instance's own [`SourceBinding::config`].
    pub sources: BTreeMap<String, serde_json::Value>,
}

/// What an environment's next revision will be: its manifest, the module manifests it lists
/// and the vars a commit freezes. Editable; [`commit`](crate::Store::commit_draft)ting it makes
/// a [`Revision`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Draft {
    /// The environment manifest. Stored without placement, as `env.toml` is.
    pub manifest: EnvManifest,
    /// The texts of the `module.yaml` files the manifest lists (`[catalog] manifests`), by
    /// the path it lists them under.
    pub modules: BTreeMap<String, String>,
    /// Vars the revision carries.
    pub vars: VarSet,
}

impl Draft {
    /// A draft of `manifest` with no module manifests or vars.
    pub fn new(manifest: EnvManifest) -> Self {
        Draft {
            manifest,
            modules: BTreeMap::new(),
            vars: VarSet::new(),
        }
    }
}

/// An environment of a workspace: its editable draft, the vars all its revisions see, and its
/// latest revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Environment {
    /// Its id.
    pub id: EnvironmentId,
    /// Its workspace.
    pub workspace: WorkspaceId,
    /// Its name (the manifest's `env_name`).
    pub name: String,
    /// What the next revision will be.
    pub draft: Draft,
    /// Vars every revision of the environment sees.
    pub vars: VarSet,
    /// The number of its latest revision, if it has one.
    pub head: Option<u32>,
    /// How many times its draft was edited: what
    /// [`update_draft_if`](crate::Store::update_draft_if) checks to refuse overwriting an edit
    /// made since the draft was read.
    pub draft_version: u64,
}

/// A numbered, immutable snapshot of an environment: the placement-free manifest, the module
/// manifests it lists and its vars, with a digest of all three, plus when it was committed and
/// with what message. Nothing in the API changes a revision once committed; the type has no
/// setters, and the store refuses an update.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Revision {
    pub(crate) environment: EnvironmentId,
    pub(crate) number: u32,
    pub(crate) manifest: EnvManifest,
    pub(crate) manifest_toml: String,
    pub(crate) modules: BTreeMap<String, String>,
    pub(crate) vars: VarSet,
    pub(crate) digest: String,
    pub(crate) message: Option<String>,
    pub(crate) created_at: SystemTime,
}

impl Revision {
    /// The environment it is a revision of.
    pub fn environment(&self) -> &EnvironmentId {
        &self.environment
    }

    /// Its number: 1 for the first, counting up per environment.
    pub fn number(&self) -> u32 {
        self.number
    }

    /// The environment manifest, without placement.
    pub fn manifest(&self) -> &EnvManifest {
        &self.manifest
    }

    /// The manifest as stored (`env.toml` text).
    pub fn manifest_toml(&self) -> &str {
        &self.manifest_toml
    }

    /// The `module.yaml` texts the manifest lists, by path.
    pub fn modules(&self) -> &BTreeMap<String, String> {
        &self.modules
    }

    /// The vars it carries.
    pub fn vars(&self) -> &VarSet {
        &self.vars
    }

    /// The SHA-256 (hex) over the manifest text, the module texts and the vars.
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// The message it was committed with, if any (see
    /// [`commit_draft_with`](crate::Store::commit_draft_with)). Not part of the digest.
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    /// When it was committed, to the second.
    pub fn created_at(&self) -> SystemTime {
        self.created_at
    }
}

/// The digest of a revision's content (see [`Revision::digest`]).
pub(crate) fn digest(
    manifest_toml: &str,
    modules: &BTreeMap<String, String>,
    vars: &VarSet,
) -> Result<String, serde_json::Error> {
    #[derive(Serialize)]
    struct Content<'a> {
        manifest: &'a str,
        modules: &'a BTreeMap<String, String>,
        vars: &'a VarSet,
    }
    let bytes = serde_json::to_vec(&Content {
        manifest: manifest_toml,
        modules,
        vars,
    })?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

/// Whether an instance should be running.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Desired {
    /// Running.
    Running,
    /// Stopped.
    Stopped,
}

impl Desired {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Desired::Running => "running",
            Desired::Stopped => "stopped",
        }
    }

    pub(crate) fn parse(s: &str) -> Self {
        if s == "running" {
            Desired::Running
        } else {
            Desired::Stopped
        }
    }
}

/// A revision as something to run: which one, whether it should be running, the overrides on
/// top of its vars and the sources whose overlays it takes. Where it runs is its
/// [`SlotLease`] (none until it is leased one).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instance {
    /// Its id.
    pub id: InstanceId,
    /// The attached store holding its environment, or `None` for the local one.
    pub store: Option<AttachmentId>,
    /// The environment.
    pub environment: EnvironmentId,
    /// The revision it runs.
    pub revision: u32,
    /// Whether it should be running.
    pub desired: Desired,
    /// Vars that override every other layer.
    pub overrides: VarSet,
    /// The overlay sources it takes, in order.
    pub sources: Vec<SourceBinding>,
    /// Its slot, when leased.
    pub slot: Option<u16>,
}

/// A slot held by an instance, host-wide: no two instances on the host share one, whatever
/// their workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotLease {
    /// The slot.
    pub slot: u16,
    /// Who holds it.
    pub instance: InstanceId,
}
