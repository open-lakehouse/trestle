//! [`Store`]: the SQLite database the CLI and the desktop app share.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use olai_stack_topology::workspace::is_valid_name;
use olai_stack_topology::{EnvManifest, MAX_SLOT};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

use crate::error::{Error, Result};
use crate::model::{
    Desired, Draft, Environment, EnvironmentId, Instance, InstanceId, Revision, SlotLease, VarSet,
    Workspace, WorkspaceId, digest,
};

/// The environment variable naming the trestle state directory.
pub const TRESTLE_HOME_VAR: &str = "TRESTLE_HOME";

/// The database file inside the state directory.
pub const DB_FILE: &str = "state.db";

/// The migrations, in order: migration `n` (1-based) takes the schema from version `n - 1` to
/// `n`.
const MIGRATIONS: &[&str] = &[include_str!("../migrations/0001_init.sql")];

/// The schema version this crate writes and reads.
pub const SCHEMA_VERSION: u32 = MIGRATIONS.len() as u32;

/// How long a writer waits for another process's write to finish.
const BUSY_TIMEOUT: Duration = Duration::from_secs(10);

/// The trestle state directory: `$TRESTLE_HOME` when set, else `trestle` in the platform's data
/// directory (e.g. `~/Library/Application Support/trestle`, `~/.local/share/trestle`).
pub fn trestle_home() -> Option<PathBuf> {
    match std::env::var_os(TRESTLE_HOME_VAR) {
        Some(home) if !home.is_empty() => Some(PathBuf::from(home)),
        _ => dirs::data_dir().map(|d| d.join("trestle")),
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

fn json<T: serde::Serialize>(value: &T) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}

fn from_json<T: serde::de::DeserializeOwned>(text: &str) -> Result<T> {
    Ok(serde_json::from_str(text)?)
}

fn check_name(name: &str) -> Result<()> {
    if is_valid_name(name) {
        Ok(())
    } else {
        Err(Error::InvalidName(name.to_owned()))
    }
}

fn is_unique_violation(e: &rusqlite::Error) -> bool {
    matches!(
        e,
        rusqlite::Error::SqliteFailure(f, _)
            if f.code == rusqlite::ErrorCode::ConstraintViolation
    )
}

/// The trestle state: workspaces, their environments (each a draft and numbered, immutable
/// revisions), instances and the host-wide slot leases, in one SQLite database (WAL mode, so
/// readers never block the writer).
///
/// Several processes open the same file: the CLI and the desktop app see each other's writes.
/// A write waits for another process's to finish. A [`Watcher`](crate::Watcher) notices commits
/// made through other connections.
pub struct Store {
    conn: Mutex<Connection>,
    path: PathBuf,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store").field("path", &self.path).finish()
    }
}

/// Open a connection to `path` with the pragmas every connection needs.
pub(crate) fn connect(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", true)?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    Ok(conn)
}

impl Store {
    /// Open (creating it if needed) the store in the [state directory](trestle_home).
    pub fn open_default() -> Result<Self> {
        let home = trestle_home().ok_or_else(|| Error::NotFound {
            kind: "data directory",
            id: format!("(set {TRESTLE_HOME_VAR})"),
        })?;
        std::fs::create_dir_all(&home).map_err(|e| Error::io(&home, e))?;
        Self::open(&home.join(DB_FILE))
    }

    /// Open (creating it if needed) the store at `path`, and bring its schema up to
    /// [`SCHEMA_VERSION`]. Errors with [`Error::UnsupportedVersion`] for a newer schema rather
    /// than read it half-way.
    pub fn open(path: &Path) -> Result<Self> {
        let mut conn = connect(path)?;
        migrate(&mut conn)?;
        Ok(Store {
            conn: Mutex::new(conn),
            path: path.to_owned(),
        })
    }

    /// The database file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn lock(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Run `f` in a write transaction, taken up front so two processes never deadlock on
    /// lock upgrades.
    fn write<T>(&self, f: impl FnOnce(&Transaction<'_>) -> Result<T>) -> Result<T> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let out = f(&tx)?;
        tx.commit()?;
        Ok(out)
    }

    /// Run `f` against a read snapshot.
    fn read<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        f(&self.lock())
    }

    // --- Workspaces --------------------------------------------------------------------------

    /// Create a workspace called `name`.
    pub fn create_workspace(&self, name: &str, mesh: bool) -> Result<Workspace> {
        check_name(name)?;
        let ws = Workspace {
            id: WorkspaceId::generate(),
            name: name.to_owned(),
            mesh,
            vars: VarSet::new(),
        };
        self.write(|tx| {
            tx.execute(
                "INSERT INTO workspace (id, name, mesh, vars, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![ws.id.0, ws.name, ws.mesh, json(&ws.vars)?, now()],
            )
            .map_err(|e| {
                if is_unique_violation(&e) {
                    Error::Duplicate {
                        kind: "workspace",
                        name: name.to_owned(),
                    }
                } else {
                    e.into()
                }
            })?;
            Ok(())
        })?;
        Ok(ws)
    }

    /// Every workspace, by name.
    pub fn workspaces(&self) -> Result<Vec<Workspace>> {
        self.read(|conn| {
            let mut stmt =
                conn.prepare("SELECT id, name, mesh, vars FROM workspace ORDER BY name")?;
            let rows = stmt.query_map([], workspace_row)?;
            rows.map(|r| r?).collect()
        })
    }

    /// The workspace called `name`.
    pub fn workspace(&self, name: &str) -> Result<Option<Workspace>> {
        self.read(|conn| {
            conn.query_row(
                "SELECT id, name, mesh, vars FROM workspace WHERE name = ?1",
                [name],
                workspace_row,
            )
            .optional()?
            .transpose()
        })
    }

    /// The workspace `id`.
    pub fn workspace_by_id(&self, id: &WorkspaceId) -> Result<Workspace> {
        self.read(|conn| {
            conn.query_row(
                "SELECT id, name, mesh, vars FROM workspace WHERE id = ?1",
                [&id.0],
                workspace_row,
            )
            .optional()?
            .transpose()?
            .ok_or_else(|| Error::NotFound {
                kind: "workspace",
                id: id.0.clone(),
            })
        })
    }

    /// Replace the workspace's vars.
    pub fn set_workspace_vars(&self, id: &WorkspaceId, vars: &VarSet) -> Result<()> {
        self.write(|tx| {
            let n = tx.execute(
                "UPDATE workspace SET vars = ?2 WHERE id = ?1",
                params![id.0, json(vars)?],
            )?;
            found(n, "workspace", &id.0)
        })
    }

    /// Delete the workspace with all its environments, revisions, instances and leases.
    pub fn delete_workspace(&self, id: &WorkspaceId) -> Result<()> {
        self.write(|tx| {
            let n = tx.execute("DELETE FROM workspace WHERE id = ?1", [&id.0])?;
            found(n, "workspace", &id.0)
        })
    }

    // --- Environments ------------------------------------------------------------------------

    /// Create the environment `name` in `workspace`, with `draft` as its draft and no
    /// revision yet. The draft's `env_name` is set to `name`.
    pub fn create_environment(
        &self,
        workspace: &WorkspaceId,
        name: &str,
        mut draft: Draft,
    ) -> Result<Environment> {
        check_name(name)?;
        draft.manifest.context.env_name = name.to_owned();
        let env = Environment {
            id: EnvironmentId::generate(),
            workspace: workspace.clone(),
            name: name.to_owned(),
            draft,
            vars: VarSet::new(),
            head: None,
        };
        self.write(|tx| {
            tx.execute(
                "INSERT INTO environment (id, workspace_id, name, draft, draft_modules, draft_vars, \
                 vars, head, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8)",
                params![
                    env.id.0,
                    workspace.0,
                    env.name,
                    env.draft.manifest.to_toml()?,
                    json(&env.draft.modules)?,
                    json(&env.draft.vars)?,
                    json(&env.vars)?,
                    now()
                ],
            )
            .map_err(|e| {
                if is_unique_violation(&e) {
                    Error::Duplicate {
                        kind: "environment",
                        name: name.to_owned(),
                    }
                } else if matches!(&e, rusqlite::Error::SqliteFailure(f, _)
                    if f.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_FOREIGNKEY)
                {
                    Error::NotFound {
                        kind: "workspace",
                        id: workspace.0.clone(),
                    }
                } else {
                    e.into()
                }
            })?;
            Ok(())
        })?;
        Ok(env)
    }

    /// The environments of `workspace`, by name.
    pub fn environments(&self, workspace: &WorkspaceId) -> Result<Vec<Environment>> {
        self.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "{ENV_COLUMNS} WHERE workspace_id = ?1 ORDER BY name"
            ))?;
            let rows = stmt.query_map([&workspace.0], environment_row)?;
            rows.map(|r| r?).collect()
        })
    }

    /// The environment `id`.
    pub fn environment(&self, id: &EnvironmentId) -> Result<Environment> {
        self.read(|conn| {
            conn.query_row(
                &format!("{ENV_COLUMNS} WHERE id = ?1"),
                [&id.0],
                environment_row,
            )
            .optional()?
            .transpose()?
            .ok_or_else(|| Error::NotFound {
                kind: "environment",
                id: id.0.clone(),
            })
        })
    }

    /// Replace the environment's draft. Its committed revisions are untouched. The draft's
    /// `env_name` is kept as the environment's name.
    pub fn update_draft(&self, id: &EnvironmentId, draft: &Draft) -> Result<()> {
        let env = self.environment(id)?;
        let mut manifest = draft.manifest.clone();
        manifest.context.env_name = env.name;
        self.write(|tx| {
            let n = tx.execute(
                "UPDATE environment SET draft = ?2, draft_modules = ?3, draft_vars = ?4 WHERE id = ?1",
                params![
                    id.0,
                    manifest.to_toml()?,
                    json(&draft.modules)?,
                    json(&draft.vars)?
                ],
            )?;
            found(n, "environment", &id.0)
        })
    }

    /// Replace the vars every revision of the environment sees.
    pub fn set_environment_vars(&self, id: &EnvironmentId, vars: &VarSet) -> Result<()> {
        self.write(|tx| {
            let n = tx.execute(
                "UPDATE environment SET vars = ?2 WHERE id = ?1",
                params![id.0, json(vars)?],
            )?;
            found(n, "environment", &id.0)
        })
    }

    /// Commit the environment's draft as its next revision (numbered one past its head) and
    /// make it the head.
    pub fn commit_draft(&self, id: &EnvironmentId) -> Result<Revision> {
        self.write(|tx| {
            let (draft, modules, vars, head): (String, String, String, Option<u32>) = tx
                .query_row(
                    "SELECT draft, draft_modules, draft_vars, head FROM environment WHERE id = ?1",
                    [&id.0],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .optional()?
                .ok_or_else(|| Error::NotFound {
                    kind: "environment",
                    id: id.0.clone(),
                })?;
            let number = head.unwrap_or(0) + 1;
            let modules: BTreeMap<String, String> = from_json(&modules)?;
            let vars: VarSet = from_json(&vars)?;
            let digest = digest(&draft, &modules, &vars)?;
            tx.execute(
                "INSERT INTO revision (environment_id, number, manifest, modules, vars, digest, \
                 created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    id.0,
                    number,
                    draft,
                    json(&modules)?,
                    json(&vars)?,
                    digest,
                    now()
                ],
            )?;
            tx.execute(
                "UPDATE environment SET head = ?2 WHERE id = ?1",
                params![id.0, number],
            )?;
            Ok(Revision {
                environment: id.clone(),
                number,
                manifest: EnvManifest::from_toml(&draft)?,
                manifest_toml: draft,
                modules,
                vars,
                digest,
            })
        })
    }

    /// Revision `number` of the environment.
    pub fn revision(&self, env: &EnvironmentId, number: u32) -> Result<Revision> {
        self.read(|conn| {
            conn.query_row(
                &format!("{REV_COLUMNS} WHERE environment_id = ?1 AND number = ?2"),
                params![env.0, number],
                revision_row,
            )
            .optional()?
            .transpose()?
            .ok_or_else(|| Error::NotFound {
                kind: "revision",
                id: format!("{env}#{number}"),
            })
        })
    }

    /// Every revision of the environment, oldest first.
    pub fn revisions(&self, env: &EnvironmentId) -> Result<Vec<Revision>> {
        self.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "{REV_COLUMNS} WHERE environment_id = ?1 ORDER BY number"
            ))?;
            let rows = stmt.query_map([&env.0], revision_row)?;
            rows.map(|r| r?).collect()
        })
    }

    // --- Instances ---------------------------------------------------------------------------

    /// Create an instance of revision `revision` of the environment, stopped and without a
    /// slot.
    pub fn create_instance(&self, env: &EnvironmentId, revision: u32) -> Result<Instance> {
        let instance = Instance {
            id: InstanceId::generate(),
            environment: env.clone(),
            revision,
            desired: Desired::Stopped,
            overrides: VarSet::new(),
            sources: Vec::new(),
            slot: None,
        };
        self.write(|tx| {
            tx.execute(
                "INSERT INTO instance (id, environment_id, revision, desired, overrides, sources, \
                 created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    instance.id.0,
                    env.0,
                    revision,
                    instance.desired.as_str(),
                    json(&instance.overrides)?,
                    json(&instance.sources)?,
                    now()
                ],
            )
            .map_err(|e| match &e {
                rusqlite::Error::SqliteFailure(f, _)
                    if f.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_FOREIGNKEY =>
                {
                    Error::NotFound {
                        kind: "revision",
                        id: format!("{env}#{revision}"),
                    }
                }
                _ => e.into(),
            })?;
            Ok(())
        })?;
        Ok(instance)
    }

    /// The instance `id`.
    pub fn instance(&self, id: &InstanceId) -> Result<Instance> {
        self.read(|conn| {
            conn.query_row(
                &format!("{INST_COLUMNS} WHERE i.id = ?1"),
                [&id.0],
                instance_row,
            )
            .optional()?
            .transpose()?
            .ok_or_else(|| Error::NotFound {
                kind: "instance",
                id: id.0.clone(),
            })
        })
    }

    /// Every instance on the host, oldest first.
    pub fn instances(&self) -> Result<Vec<Instance>> {
        self.read(|conn| {
            let mut stmt = conn.prepare(&format!("{INST_COLUMNS} ORDER BY i.created_at, i.id"))?;
            let rows = stmt.query_map([], instance_row)?;
            rows.map(|r| r?).collect()
        })
    }

    /// Set whether the instance should be running.
    pub fn set_desired(&self, id: &InstanceId, desired: Desired) -> Result<()> {
        self.write(|tx| {
            let n = tx.execute(
                "UPDATE instance SET desired = ?2 WHERE id = ?1",
                params![id.0, desired.as_str()],
            )?;
            found(n, "instance", &id.0)
        })
    }

    /// Replace the instance's overrides.
    pub fn set_overrides(&self, id: &InstanceId, overrides: &VarSet) -> Result<()> {
        self.write(|tx| {
            let n = tx.execute(
                "UPDATE instance SET overrides = ?2 WHERE id = ?1",
                params![id.0, json(overrides)?],
            )?;
            found(n, "instance", &id.0)
        })
    }

    /// Replace the overlay sources the instance takes.
    pub fn set_sources(&self, id: &InstanceId, sources: &[String]) -> Result<()> {
        self.write(|tx| {
            let n = tx.execute(
                "UPDATE instance SET sources = ?2 WHERE id = ?1",
                params![id.0, json(&sources)?],
            )?;
            found(n, "instance", &id.0)
        })
    }

    /// Delete the instance, releasing its slot.
    pub fn delete_instance(&self, id: &InstanceId) -> Result<()> {
        self.write(|tx| {
            let n = tx.execute("DELETE FROM instance WHERE id = ?1", [&id.0])?;
            found(n, "instance", &id.0)
        })
    }

    // --- Slot leases -------------------------------------------------------------------------

    /// Lease the instance the lowest slot no instance on the host holds, or return the one it
    /// already holds. Atomic across processes.
    pub fn lease_slot(&self, instance: &InstanceId) -> Result<SlotLease> {
        self.write(|tx| {
            let held: Option<u16> = tx
                .query_row(
                    "SELECT slot FROM slot_lease WHERE instance_id = ?1",
                    [&instance.0],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(slot) = held {
                return Ok(SlotLease {
                    slot,
                    instance: instance.clone(),
                });
            }
            let taken: Vec<u16> = tx
                .prepare("SELECT slot FROM slot_lease ORDER BY slot")?
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            let slot = (0..=MAX_SLOT)
                .find(|s| taken.binary_search(s).is_err())
                .ok_or(Error::SlotsExhausted)?;
            tx.execute(
                "INSERT INTO slot_lease (slot, instance_id, leased_at) VALUES (?1, ?2, ?3)",
                params![slot, instance.0, now()],
            )
            .map_err(|e| match &e {
                rusqlite::Error::SqliteFailure(f, _)
                    if f.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_FOREIGNKEY =>
                {
                    Error::NotFound {
                        kind: "instance",
                        id: instance.0.clone(),
                    }
                }
                _ => e.into(),
            })?;
            Ok(SlotLease {
                slot,
                instance: instance.clone(),
            })
        })
    }

    /// Release the instance's slot, if it holds one.
    pub fn release_slot(&self, instance: &InstanceId) -> Result<()> {
        self.write(|tx| {
            tx.execute(
                "DELETE FROM slot_lease WHERE instance_id = ?1",
                [&instance.0],
            )?;
            Ok(())
        })
    }

    /// Every lease on the host, by slot.
    pub fn leases(&self) -> Result<Vec<SlotLease>> {
        self.read(|conn| {
            let mut stmt =
                conn.prepare("SELECT slot, instance_id FROM slot_lease ORDER BY slot")?;
            let rows = stmt.query_map([], |r| {
                Ok(SlotLease {
                    slot: r.get(0)?,
                    instance: InstanceId(r.get(1)?),
                })
            })?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
    }
}

fn found(rows: usize, kind: &'static str, id: &str) -> Result<()> {
    if rows == 0 {
        Err(Error::NotFound {
            kind,
            id: id.to_owned(),
        })
    } else {
        Ok(())
    }
}

/// Bring `conn`'s schema up to [`SCHEMA_VERSION`], in one write transaction so two processes
/// opening a new store at once don't both migrate it.
fn migrate(conn: &mut Connection) -> Result<()> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let found: u32 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if found > SCHEMA_VERSION {
        return Err(Error::UnsupportedVersion {
            found,
            supported: SCHEMA_VERSION,
        });
    }
    for sql in &MIGRATIONS[found as usize..] {
        tx.execute_batch(sql)?;
    }
    tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    tx.commit()?;
    Ok(())
}

fn workspace_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Result<Workspace>> {
    let vars: String = r.get(3)?;
    let (id, name, mesh) = (r.get(0)?, r.get(1)?, r.get(2)?);
    Ok(from_json(&vars).map(|vars| Workspace {
        id: WorkspaceId(id),
        name,
        mesh,
        vars,
    }))
}

const ENV_COLUMNS: &str =
    "SELECT id, workspace_id, name, draft, draft_modules, draft_vars, vars, head FROM environment";

fn environment_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Result<Environment>> {
    let (id, workspace, name): (String, String, String) = (r.get(0)?, r.get(1)?, r.get(2)?);
    let (draft, modules, draft_vars, vars): (String, String, String, String) =
        (r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?);
    let head: Option<u32> = r.get(7)?;
    Ok((|| {
        Ok(Environment {
            id: EnvironmentId(id),
            workspace: WorkspaceId(workspace),
            name,
            draft: Draft {
                manifest: EnvManifest::from_toml(&draft)?,
                modules: from_json(&modules)?,
                vars: from_json(&draft_vars)?,
            },
            vars: from_json(&vars)?,
            head,
        })
    })())
}

const REV_COLUMNS: &str =
    "SELECT environment_id, number, manifest, modules, vars, digest FROM revision";

fn revision_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Result<Revision>> {
    let (env, number, manifest_toml): (String, u32, String) = (r.get(0)?, r.get(1)?, r.get(2)?);
    let (modules, vars, digest): (String, String, String) = (r.get(3)?, r.get(4)?, r.get(5)?);
    Ok((|| {
        Ok(Revision {
            environment: EnvironmentId(env),
            number,
            manifest: EnvManifest::from_toml(&manifest_toml)?,
            manifest_toml,
            modules: from_json(&modules)?,
            vars: from_json(&vars)?,
            digest,
        })
    })())
}

const INST_COLUMNS: &str = "SELECT i.id, i.environment_id, i.revision, i.desired, i.overrides, \
     i.sources, l.slot FROM instance i LEFT JOIN slot_lease l ON l.instance_id = i.id";

fn instance_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Result<Instance>> {
    let (id, env, revision, desired): (String, String, u32, String) =
        (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?);
    let (overrides, sources, slot): (String, String, Option<u16>) =
        (r.get(4)?, r.get(5)?, r.get(6)?);
    Ok((|| {
        Ok(Instance {
            id: InstanceId(id),
            environment: EnvironmentId(env),
            revision,
            desired: Desired::parse(&desired),
            overrides: from_json(&overrides)?,
            sources: from_json(&sources)?,
            slot,
        })
    })())
}
