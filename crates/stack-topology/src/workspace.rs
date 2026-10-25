//! Several environments on one host: the [`WorkspaceManifest`].
//!
//! A workspace is a directory holding a `trestle-workspace.toml` and one environment directory
//! per member. Each member stays a self-contained environment (its own [`EnvManifest`]
//! (crate::EnvManifest), its own compose project); the workspace only decides the values that
//! must differ between environments sharing a Docker host, and checks them:
//!
//! - a **slot** per environment, which shifts every host port the environment binds by
//!   `slot × `[`SLOT_PORT_STRIDE`] (slot 0 keeps the single-environment defaults);
//! - the compose **project name**, `<workspace>-<env>`, so equally named environments in two
//!   workspaces never share containers or volumes;
//! - optionally a **mesh** network every member's gateway joins (see
//!   [`PlanCtx::mesh_network`]).
//!
//! [`apply`](WorkspaceManifest::apply) writes a member's values into its [`PlanCtx`];
//! [`port_conflicts`] checks the planned members against each other.
//!
//! Pure and WASM-clean, like [`manifest`](crate::manifest): (de)serialization works on strings,
//! and only [`read_from`](WorkspaceManifest::read_from) / [`write_to`](WorkspaceManifest::write_to)
//! touch the filesystem, behind the `std-io` feature.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::plan::{Plan, PlanCtx};

/// The file a workspace directory is recognised by.
pub const WORKSPACE_FILE: &str = "trestle-workspace.toml";

/// How far apart two adjacent slots' host ports are: slot `n` binds every port `n × 1000`
/// above slot 0's.
pub const SLOT_PORT_STRIDE: u16 = 1000;

/// The highest slot a workspace hands out, so a shifted default port (the highest is the
/// Envoy admin port, `9901`) stays below `65535`.
pub const MAX_SLOT: u16 = 54;

/// What can go wrong reading, editing or checking a [`WorkspaceManifest`].
#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    /// Serializing the manifest to TOML failed.
    #[error("serializing the workspace manifest to TOML failed: {0}")]
    Serialize(#[from] toml::ser::Error),
    /// Parsing the manifest from TOML failed.
    #[error("parsing the workspace manifest from TOML failed: {0}")]
    Deserialize(#[from] toml::de::Error),
    /// The manifest was written by a newer version than this one understands.
    #[error(
        "workspace manifest version {found} is newer than this version supports ({supported}); \
         upgrade trestle"
    )]
    UnsupportedVersion {
        /// The file's `version`.
        found: u32,
        /// The newest version this crate reads.
        supported: u32,
    },
    /// A workspace or environment name is not a valid name (see [`is_valid_name`]).
    #[error(
        "`{0}` is not a valid name: use lowercase letters, digits and dashes, starting with a letter"
    )]
    InvalidName(String),
    /// The workspace already has an environment of that name.
    #[error("the workspace already has an environment `{0}`")]
    DuplicateEnv(String),
    /// The workspace has no environment of that name.
    #[error("the workspace has no environment `{0}`")]
    UnknownEnv(String),
    /// Two environments hold the same slot.
    #[error("environments `{first}` and `{second}` both hold slot {slot}")]
    DuplicateSlot {
        /// The shared slot.
        slot: u16,
        /// The first holder.
        first: String,
        /// The second holder.
        second: String,
    },
    /// A slot beyond [`MAX_SLOT`], or every slot is taken.
    #[error("no free slot: a workspace holds at most {} environments", MAX_SLOT + 1)]
    SlotsExhausted,
    /// Reading or writing the manifest file failed (only the `std-io` helpers).
    #[cfg(feature = "std-io")]
    #[error("reading/writing the workspace manifest file failed: {0}")]
    Io(#[from] std::io::Error),
}

/// Whether `name` is usable as a workspace or environment name: `^[a-z][a-z0-9-]*$`. The name
/// becomes a directory, a compose project and a DNS label, so nothing else is allowed.
pub fn is_valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && name.len() <= 63
}

/// The persisted record of a workspace: its name, whether its gateways share a mesh network,
/// and its member environments. See the [module docs](self).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceManifest {
    /// The manifest schema version.
    #[serde(default = "WorkspaceManifest::current_version")]
    pub version: u32,
    /// The workspace's name: the prefix of every member's compose project, and of the mesh
    /// network's name.
    pub name: String,
    /// Whether every member's gateway joins the shared [`mesh_network`](Self::mesh_network).
    #[serde(default)]
    pub mesh: bool,
    /// The member environments.
    #[serde(default)]
    pub envs: Vec<WorkspaceEnv>,
}

/// One member environment of a [`WorkspaceManifest`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceEnv {
    /// The environment's name (its [`PlanCtx::env_name`]).
    pub name: String,
    /// The environment's directory, relative to the workspace directory.
    pub path: String,
    /// The environment's slot: its host ports sit `slot × `[`SLOT_PORT_STRIDE`] above the
    /// single-environment defaults.
    pub slot: u16,
}

impl WorkspaceEnv {
    /// The amount this environment's host ports are shifted by.
    pub fn port_offset(&self) -> u16 {
        self.slot.saturating_mul(SLOT_PORT_STRIDE)
    }
}

impl WorkspaceManifest {
    /// The current manifest schema version.
    pub fn current_version() -> u32 {
        1
    }

    /// An empty workspace called `name`.
    pub fn new(name: impl Into<String>, mesh: bool) -> Result<Self, WorkspaceError> {
        let name = name.into();
        if !is_valid_name(&name) {
            return Err(WorkspaceError::InvalidName(name));
        }
        Ok(WorkspaceManifest {
            version: Self::current_version(),
            name,
            mesh,
            envs: Vec::new(),
        })
    }

    /// Serialize this manifest to a TOML string.
    pub fn to_toml(&self) -> Result<String, WorkspaceError> {
        Ok(toml::to_string_pretty(self)?)
    }

    /// Parse a manifest from a TOML string, and [`validate`](Self::validate) it.
    pub fn from_toml(s: &str) -> Result<Self, WorkspaceError> {
        let manifest: Self = toml::from_str(s)?;
        if manifest.version > Self::current_version() {
            return Err(WorkspaceError::UnsupportedVersion {
                found: manifest.version,
                supported: Self::current_version(),
            });
        }
        manifest.validate()?;
        Ok(manifest)
    }

    /// Check the names are valid and that no two members share a name or a slot.
    pub fn validate(&self) -> Result<(), WorkspaceError> {
        if !is_valid_name(&self.name) {
            return Err(WorkspaceError::InvalidName(self.name.clone()));
        }
        let mut slots: BTreeMap<u16, &str> = BTreeMap::new();
        for (idx, env) in self.envs.iter().enumerate() {
            if !is_valid_name(&env.name) {
                return Err(WorkspaceError::InvalidName(env.name.clone()));
            }
            if self.envs[..idx].iter().any(|e| e.name == env.name) {
                return Err(WorkspaceError::DuplicateEnv(env.name.clone()));
            }
            if env.slot > MAX_SLOT {
                return Err(WorkspaceError::SlotsExhausted);
            }
            if let Some(first) = slots.insert(env.slot, &env.name) {
                return Err(WorkspaceError::DuplicateSlot {
                    slot: env.slot,
                    first: first.into(),
                    second: env.name.clone(),
                });
            }
        }
        Ok(())
    }

    /// The member called `name`.
    pub fn env(&self, name: &str) -> Option<&WorkspaceEnv> {
        self.envs.iter().find(|e| e.name == name)
    }

    /// The lowest slot no member holds.
    pub fn next_slot(&self) -> Result<u16, WorkspaceError> {
        (0..=MAX_SLOT)
            .find(|slot| self.envs.iter().all(|e| e.slot != *slot))
            .ok_or(WorkspaceError::SlotsExhausted)
    }

    /// Add a member called `name`, in the directory of the same name, on the
    /// [next free slot](Self::next_slot).
    pub fn add_env(&mut self, name: impl Into<String>) -> Result<&WorkspaceEnv, WorkspaceError> {
        let name = name.into();
        if !is_valid_name(&name) {
            return Err(WorkspaceError::InvalidName(name));
        }
        if self.env(&name).is_some() {
            return Err(WorkspaceError::DuplicateEnv(name));
        }
        let slot = self.next_slot()?;
        self.envs.push(WorkspaceEnv {
            path: name.clone(),
            name,
            slot,
        });
        Ok(self.envs.last().expect("just pushed"))
    }

    /// Drop the member called `name`, freeing its slot.
    pub fn remove_env(&mut self, name: &str) -> Result<WorkspaceEnv, WorkspaceError> {
        let idx = self
            .envs
            .iter()
            .position(|e| e.name == name)
            .ok_or_else(|| WorkspaceError::UnknownEnv(name.into()))?;
        Ok(self.envs.remove(idx))
    }

    /// The compose project name of `env`: `<workspace>-<env>`.
    pub fn project_name(&self, env: &WorkspaceEnv) -> String {
        format!("{}-{}", self.name, env.name)
    }

    /// The shared network the members' gateways join, when [`mesh`](Self::mesh) is on:
    /// `trestle-<workspace>-mesh`.
    pub fn mesh_network(&self) -> Option<String> {
        self.mesh.then(|| format!("trestle-{}-mesh", self.name))
    }

    /// Write what the workspace decides for `env` into its `ctx`: the env and project names,
    /// the mesh network, the [`port_offset`](PlanCtx::port_offset) and the gateway, admin and
    /// dedicated-listener ports, each the single-environment default shifted by the slot. An
    /// explicit [`dedicated_listener_ports`](PlanCtx::dedicated_listener_ports) list is kept
    /// only if it lies between the slot's dedicated-listener base and its admin port, so a
    /// member moved to another slot doesn't keep pins into its old one.
    pub fn apply(&self, env: &WorkspaceEnv, ctx: &mut PlanCtx) {
        let defaults = PlanCtx::default();
        let offset = env.port_offset();
        ctx.env_name = env.name.clone();
        ctx.project_name = Some(self.project_name(env));
        ctx.mesh_network = self.mesh_network();
        ctx.port_offset = offset;
        ctx.gateway_host_port = defaults.gateway_host_port.saturating_add(offset);
        ctx.gateway_admin_port = defaults.gateway_admin_port.saturating_add(offset);
        ctx.dedicated_listener_port_base =
            defaults.dedicated_listener_port_base.saturating_add(offset);
        let slot_range = ctx.dedicated_listener_port_base..ctx.gateway_admin_port;
        if !ctx
            .dedicated_listener_ports
            .iter()
            .all(|p| slot_range.contains(p))
        {
            ctx.dedicated_listener_ports.clear();
        }
    }
}

#[cfg(feature = "std-io")]
impl WorkspaceManifest {
    /// Read and parse a manifest from `path`. Behind `std-io`.
    pub fn read_from(path: &std::path::Path) -> Result<Self, WorkspaceError> {
        Self::from_toml(&std::fs::read_to_string(path)?)
    }

    /// Serialize this manifest and write it to `path`, creating parent directories as needed.
    /// Behind `std-io`.
    pub fn write_to(&self, path: &std::path::Path) -> Result<(), WorkspaceError> {
        let toml = self.to_toml()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, toml)?;
        Ok(())
    }
}

/// A host port two environments both bind (see [`port_conflicts`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PortConflict {
    /// The doubly bound port.
    pub port: u16,
    /// The environment that binds it first (in the order given), and its owner there (see
    /// [`Plan::host_ports`]).
    pub first: (String, String),
    /// The other environment, and its owner there.
    pub second: (String, String),
}

impl std::fmt::Display for PortConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "host port {} is bound by `{}` in `{}` and by `{}` in `{}`",
            self.port, self.first.1, self.first.0, self.second.1, self.second.0
        )
    }
}

/// Every host port bound by more than one of the planned environments, as `(env name, plan)`
/// pairs, sorted by port. Empty when the environments can run side by side.
pub fn port_conflicts<'a>(
    envs: impl IntoIterator<Item = (&'a str, &'a Plan)>,
) -> Vec<PortConflict> {
    let mut owners: BTreeMap<u16, (String, String)> = BTreeMap::new();
    let mut conflicts = Vec::new();
    for (env, plan) in envs {
        for (port, owner) in plan.host_ports() {
            match owners.get(&port) {
                Some(first) if first.0 != env => conflicts.push(PortConflict {
                    port,
                    first: first.clone(),
                    second: (env.to_owned(), owner),
                }),
                Some(_) => {}
                None => {
                    owners.insert(port, (env.to_owned(), owner));
                }
            }
        }
    }
    conflicts.sort_by_key(|c| c.port);
    conflicts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Selection, baseline_catalog};

    fn demo() -> WorkspaceManifest {
        let mut ws = WorkspaceManifest::new("demo", true).unwrap();
        ws.add_env("dev").unwrap();
        ws.add_env("prod").unwrap();
        ws
    }

    fn plan_member(ws: &WorkspaceManifest, name: &str, selection: &Selection) -> Plan {
        let mut ctx = PlanCtx::default();
        ws.apply(ws.env(name).unwrap(), &mut ctx);
        baseline_catalog().plan(selection, &ctx).unwrap()
    }

    #[test]
    fn slots_are_allocated_lowest_free_first() {
        let mut ws = demo();
        assert_eq!(ws.env("dev").unwrap().slot, 0);
        assert_eq!(ws.env("prod").unwrap().slot, 1);
        ws.remove_env("dev").unwrap();
        assert_eq!(ws.add_env("staging").unwrap().slot, 0);
        assert!(matches!(
            ws.add_env("prod"),
            Err(WorkspaceError::DuplicateEnv(_))
        ));
        assert!(matches!(
            ws.add_env("Prod"),
            Err(WorkspaceError::InvalidName(_))
        ));
    }

    #[test]
    fn toml_round_trips_and_rejects_shared_slots() {
        let ws = demo();
        let toml = ws.to_toml().unwrap();
        assert_eq!(WorkspaceManifest::from_toml(&toml).unwrap(), ws);

        let clash = toml.replace("slot = 1", "slot = 0");
        assert!(matches!(
            WorkspaceManifest::from_toml(&clash),
            Err(WorkspaceError::DuplicateSlot { slot: 0, .. })
        ));
        let newer = toml.replace("version = 1", "version = 99");
        assert!(matches!(
            WorkspaceManifest::from_toml(&newer),
            Err(WorkspaceError::UnsupportedVersion { found: 99, .. })
        ));
    }

    #[test]
    fn apply_shifts_ports_and_names_the_project() {
        let ws = demo();
        let mut ctx = PlanCtx::default();
        ws.apply(ws.env("prod").unwrap(), &mut ctx);
        assert_eq!(ctx.env_name, "prod");
        assert_eq!(ctx.compose_project(), "demo-prod");
        assert_eq!(ctx.mesh_network.as_deref(), Some("trestle-demo-mesh"));
        assert_eq!(ctx.port_offset, 1000);
        assert_eq!(ctx.gateway_host_port, 10080);
        assert_eq!(ctx.gateway_admin_port, 10901);
        assert_eq!(ctx.dedicated_listener_port_base, 10100);

        // Slot 0 is the single-environment layout.
        let mut ctx = PlanCtx::default();
        ws.apply(ws.env("dev").unwrap(), &mut ctx);
        assert_eq!(ctx.gateway_host_port, PlanCtx::default().gateway_host_port);
        assert_eq!(ctx.port_offset, 0);
    }

    #[test]
    fn apply_drops_dedicated_pins_from_another_slot() {
        let ws = demo();
        let mut ctx = PlanCtx {
            dedicated_listener_ports: vec![9100, 9101],
            ..PlanCtx::default()
        };
        ws.apply(ws.env("prod").unwrap(), &mut ctx);
        assert!(ctx.dedicated_listener_ports.is_empty());

        let mut ctx = PlanCtx {
            dedicated_listener_ports: vec![10100],
            ..PlanCtx::default()
        };
        ws.apply(ws.env("prod").unwrap(), &mut ctx);
        assert_eq!(ctx.dedicated_listener_ports, vec![10100]);
    }

    #[test]
    fn members_run_side_by_side() {
        let ws = demo();
        let selection = Selection::modules(["envoy", "rustfs", "unity-catalog", "puppygraph"]);
        let dev = plan_member(&ws, "dev", &selection);
        let prod = plan_member(&ws, "prod", &selection);
        assert!(port_conflicts([("dev", &dev), ("prod", &prod)]).is_empty());

        // PuppyGraph's own ports follow the slot.
        let ports: Vec<u16> = prod.host_ports().into_iter().map(|(p, _)| p).collect();
        assert!(ports.contains(&8687), "bolt shifted: {ports:?}");
        assert!(ports.contains(&9182), "gremlin shifted: {ports:?}");
        assert!(ports.contains(&10080), "gateway: {ports:?}");

        // Two unmanaged environments with the defaults collide.
        let plain = baseline_catalog()
            .plan(&selection, &PlanCtx::default())
            .unwrap();
        let conflicts = port_conflicts([("a", &plain), ("b", &plain)]);
        assert!(conflicts.iter().any(|c| c.port == 9080));
        assert!(conflicts.iter().any(|c| c.port == 7687));
    }

    #[test]
    fn direct_publishes_follow_the_slot() {
        let ws = demo();
        let mut selection = Selection::modules(["postgres", "jaeger"]);
        selection.gateway.enabled = false;
        let dev = plan_member(&ws, "dev", &selection);
        let prod = plan_member(&ws, "prod", &selection);
        assert!(port_conflicts([("dev", &dev), ("prod", &prod)]).is_empty());
        let shifted: Vec<u16> = dev
            .host_ports()
            .into_iter()
            .map(|(p, _)| p + SLOT_PORT_STRIDE)
            .collect();
        let prod_ports: Vec<u16> = prod.host_ports().into_iter().map(|(p, _)| p).collect();
        assert_eq!(shifted, prod_ports);
        // Nothing fronts a gateway-less env, so it joins no mesh.
        assert!(prod.bindings.values().all(|b| b.mesh.is_none()));
    }

    #[test]
    fn project_name_and_mesh_render_into_compose() {
        let ws = demo();
        let plan = plan_member(&ws, "prod", &Selection::modules(["envoy", "rustfs"]));
        let out = plan.materialize().unwrap();
        let file = |path: &str| {
            out.files
                .iter()
                .find(|f| f.path == path)
                .unwrap_or_else(|| panic!("no {path}"))
                .contents
                .clone()
        };
        assert!(file("compose.yaml").contains("name: demo-prod"));
        let gateway = file("modules/envoy/compose.yaml");
        assert!(gateway.contains("name: trestle-demo-mesh"), "{gateway}");
        assert!(gateway.contains("- prod.mesh"), "{gateway}");
        assert!(gateway.contains("\"10080:10000\""), "{gateway}");
    }
}
