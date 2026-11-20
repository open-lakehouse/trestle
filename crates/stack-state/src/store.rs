//! [`Store`]: the state the CLI and the desktop app share, on this machine or a team's.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
#[cfg(feature = "postgres")]
use std::sync::Arc;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use olai_stack_topology::workspace::is_valid_name;
use olai_stack_topology::{EnvManifest, MAX_SLOT};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

use crate::error::{Error, Result};
use crate::model::{
    Attachment, AttachmentId, Desired, Draft, Environment, EnvironmentId, Instance, InstanceId,
    RemoteLocation, Revision, SlotLease, SourceBinding, VarScope, VarSet, Workspace, WorkspaceId,
    digest,
};
#[cfg(feature = "postgres")]
use crate::pg::{self, PgSession};
use crate::sql::{DbError, Row, Sql, Sqlite, args};

/// The environment variable naming the trestle state directory.
pub const TRESTLE_HOME_VAR: &str = "TRESTLE_HOME";

/// The database file inside the state directory.
pub const DB_FILE: &str = "state.db";

/// The migrations, in order: migration `n` (1-based) takes the schema from version `n - 1` to
/// `n`.
const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/0001_init.sql"),
    include_str!("../migrations/0002_revision_message.sql"),
    include_str!("../migrations/0003_local_split.sql"),
];

/// The schema version of the local `state.db` this crate writes and reads.
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

/// A `created_at` column (seconds since the epoch) as a time.
fn time_of(secs: i64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(secs.max(0) as u64)
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

/// `e`, or [`Error::Duplicate`] when it is a unique-constraint violation on `name`.
fn duplicate_or(e: DbError, kind: &'static str, name: &str) -> Error {
    match e {
        DbError::Unique(_) => Error::Duplicate {
            kind,
            name: name.to_owned(),
        },
        e => e.into(),
    }
}

fn is_unique_violation(e: &rusqlite::Error) -> bool {
    matches!(
        e,
        rusqlite::Error::SqliteFailure(f, _)
            if f.code == rusqlite::ErrorCode::ConstraintViolation
    )
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Refuse with [`Error::HasInstances`] when `instances` isn't zero.
fn refuse_instances(instances: usize, kind: &'static str, name: &str) -> Result<()> {
    if instances == 0 {
        Ok(())
    } else {
        Err(Error::HasInstances {
            kind,
            name: name.to_owned(),
            instances,
        })
    }
}

/// Where the shared tables (workspaces, environments, revisions) are.
enum Shared {
    /// In the local `state.db`, on a connection of their own.
    Sqlite(Mutex<Connection>),
    /// In a team's Postgres database.
    #[cfg(feature = "postgres")]
    Postgres(Arc<Mutex<PgSession>>),
}

impl Shared {
    /// Run `f` in a write transaction. On SQLite it takes the write lock up front, so two
    /// processes never deadlock on lock upgrades; on Postgres, rows `f` selects `FOR UPDATE`
    /// stay locked until it returns.
    fn write<T, E: From<Error>>(
        &self,
        f: impl FnOnce(&mut dyn Sql) -> std::result::Result<T, E>,
    ) -> std::result::Result<T, E> {
        match self {
            Shared::Sqlite(conn) => {
                let mut conn = lock(conn);
                let tx = conn
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(Error::from)?;
                let out = f(&mut Sqlite::tx(&tx))?;
                tx.commit().map_err(Error::from)?;
                Ok(out)
            }
            #[cfg(feature = "postgres")]
            Shared::Postgres(session) => {
                let session = lock(session);
                session.begin()?;
                let mut sql: &PgSession = &session;
                match f(&mut sql) {
                    Ok(out) => {
                        session.end(true)?;
                        Ok(out)
                    }
                    Err(e) => {
                        let _ = session.end(false);
                        Err(e)
                    }
                }
            }
        }
    }

    /// Run `f`'s statements, each on its own.
    fn read<T>(&self, f: impl FnOnce(&mut dyn Sql) -> Result<T>) -> Result<T> {
        match self {
            Shared::Sqlite(conn) => f(&mut Sqlite(&lock(conn))),
            #[cfg(feature = "postgres")]
            Shared::Postgres(session) => {
                let session = lock(session);
                let mut sql: &PgSession = &session;
                f(&mut sql)
            }
        }
    }
}

/// The trestle state.
///
/// - **Shared**: workspaces, their environments (each a draft and numbered, immutable
///   revisions) and their vars. In the local `state.db` ([`open`](Self::open)), or in a
///   team's Postgres database attached on this machine ([`open_attached`](Self::open_attached),
///   feature `postgres`), where colleagues see each other's edits.
/// - **Machine-local**, always in `state.db`: instances, the host-wide slot leases, this
///   machine's own vars ([`VarScope::Local`]) and the attachments.
///
/// Several processes open the same `state.db` (WAL mode, so readers never block the writer):
/// the CLI and the desktop app see each other's writes. A write waits for another process's
/// to finish. A [`Watcher`](crate::Watcher) notices commits made through other connections.
///
/// An instance of an attached store's revision names the attachment, so whichever store a
/// caller opens lists every instance on the host.
pub struct Store {
    local: Mutex<Connection>,
    shared: Shared,
    path: PathBuf,
    attachment: Option<AttachmentId>,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("path", &self.path)
            .field("attachment", &self.attachment)
            .finish()
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

/// The local connection to `path`, its schema brought up to [`SCHEMA_VERSION`].
fn open_local(path: &Path) -> Result<Connection> {
    let mut conn = connect(path)?;
    migrate(&mut conn)?;
    Ok(conn)
}

impl Store {
    /// Open (creating it if needed) the store in the [state directory](trestle_home).
    pub fn open_default() -> Result<Self> {
        Self::open(&default_path()?)
    }

    /// Open (creating it if needed) the store at `path`, and bring its schema up to
    /// [`SCHEMA_VERSION`]. Errors with [`Error::UnsupportedVersion`] for a newer schema rather
    /// than read it half-way.
    pub fn open(path: &Path) -> Result<Self> {
        let local = open_local(path)?;
        Ok(Store {
            local: Mutex::new(local),
            shared: Shared::Sqlite(Mutex::new(connect(path)?)),
            path: path.to_owned(),
            attachment: None,
        })
    }

    /// Open the attached store `attachment` of the local store at `path`: its shared tables
    /// in the attachment's database, reached at the connection string `resolve` returns for
    /// its location (with the user's own login: the record holds no credential), and the
    /// machine-local ones in `path`.
    ///
    /// Opening checks the database's schema version and never migrates it:
    /// [`Error::Uninitialized`] when it has none ([`init_remote`](Self::init_remote) creates
    /// it), [`Error::Outdated`] or [`Error::UnsupportedVersion`] when it isn't this crate's.
    #[cfg(feature = "postgres")]
    pub fn open_attached<E>(
        path: &Path,
        attachment: &AttachmentId,
        resolve: impl FnOnce(&RemoteLocation) -> std::result::Result<String, E>,
    ) -> Result<Self>
    where
        E: Into<Box<dyn std::error::Error + Send + Sync>>,
    {
        let local = open_local(path)?;
        let location = attachment_row(&local, attachment)?.location;
        let url = resolve(&location).map_err(|e| Error::Remote(e.into()))?;
        let session = PgSession::connect(&url)?;
        pg::check_version(&session)?;
        Ok(Store {
            local: Mutex::new(local),
            shared: Shared::Postgres(Arc::new(Mutex::new(session))),
            path: path.to_owned(),
            attachment: Some(attachment.clone()),
        })
    }

    /// Create the shared schema in the Postgres database at `url` (a connection string), or
    /// migrate it to this crate's version. Safe to run more than once, and from two
    /// processes at once. Refuses a newer schema with [`Error::UnsupportedVersion`].
    #[cfg(feature = "postgres")]
    pub fn init_remote(url: &str) -> Result<()> {
        pg::migrate(&PgSession::connect(url)?)
    }

    /// The local database file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The attachment whose database holds this store's shared tables, or `None` when they
    /// are in the local file.
    pub fn attachment_id(&self) -> Option<&AttachmentId> {
        self.attachment.as_ref()
    }

    #[cfg(feature = "postgres")]
    pub(crate) fn remote_session(&self) -> Option<Arc<Mutex<PgSession>>> {
        match &self.shared {
            Shared::Postgres(session) => Some(Arc::clone(session)),
            Shared::Sqlite(_) => None,
        }
    }

    /// Run `f` in a write transaction on the local file, taken up front so two processes
    /// never deadlock on lock upgrades.
    fn local_write<T>(&self, f: impl FnOnce(&Transaction<'_>) -> Result<T>) -> Result<T> {
        let mut conn = lock(&self.local);
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let out = f(&tx)?;
        tx.commit()?;
        Ok(out)
    }

    /// Run `f` against the local file.
    fn local_read<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        f(&lock(&self.local))
    }

    /// How many of this machine's instances run `envs`.
    fn instances_of_envs(&self, envs: &[EnvironmentId]) -> Result<usize> {
        self.local_read(|conn| {
            let mut stmt =
                conn.prepare_cached("SELECT COUNT(*) FROM instance WHERE environment_id = ?1")?;
            let mut total = 0;
            for env in envs {
                total += stmt.query_row([&env.0], |r| r.get::<_, usize>(0))?;
            }
            Ok(total)
        })
    }

    /// The ids of the workspace's environments.
    fn environment_ids(&self, workspace: &WorkspaceId) -> Result<Vec<EnvironmentId>> {
        self.shared.read(|sql| {
            sql.query(
                "SELECT id FROM environment WHERE workspace_id = $1",
                args![&workspace.0],
            )?
            .iter()
            .map(|r| r.text(0).map(EnvironmentId))
            .collect()
        })
    }

    /// Drop this machine's instances (with their leases) and own vars of `envs`, once the
    /// environments are gone.
    fn forget_environments(&self, envs: &[EnvironmentId]) -> Result<()> {
        self.local_write(|tx| {
            for env in envs {
                tx.execute("DELETE FROM instance WHERE environment_id = ?1", [&env.0])?;
                tx.execute("DELETE FROM local_var WHERE environment_id = ?1", [&env.0])?;
            }
            Ok(())
        })
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
            sources: BTreeMap::new(),
        };
        self.shared.write(|sql| {
            sql.execute(
                "INSERT INTO workspace (id, name, mesh, vars, sources, created_at) \
                 VALUES ($1, $2, $3, $4, $5, $6)",
                args![
                    &ws.id.0,
                    &ws.name,
                    ws.mesh,
                    json(&ws.vars)?,
                    json(&ws.sources)?,
                    now()
                ],
            )
            .map_err(|e| duplicate_or(e, "workspace", name))?;
            Ok::<_, Error>(())
        })?;
        Ok(ws)
    }

    /// Every workspace, by name.
    pub fn workspaces(&self) -> Result<Vec<Workspace>> {
        self.shared.read(|sql| {
            sql.query(&format!("{WS_COLUMNS} ORDER BY name"), vec![])?
                .iter()
                .map(workspace_row)
                .collect()
        })
    }

    /// The workspace called `name`.
    pub fn workspace(&self, name: &str) -> Result<Option<Workspace>> {
        self.shared.read(|sql| {
            sql.query_opt(&format!("{WS_COLUMNS} WHERE name = $1"), args![name])?
                .as_ref()
                .map(workspace_row)
                .transpose()
        })
    }

    /// The workspace `id`.
    pub fn workspace_by_id(&self, id: &WorkspaceId) -> Result<Workspace> {
        self.shared.read(|sql| {
            sql.query_opt(&format!("{WS_COLUMNS} WHERE id = $1"), args![&id.0])?
                .as_ref()
                .map(workspace_row)
                .transpose()?
                .ok_or_else(|| not_found("workspace", &id.0))
        })
    }

    /// Rename the workspace to `name`. Its id, environments, revisions, vars and source config
    /// stay as they are. Renaming it to its own name does nothing.
    ///
    /// Refused with [`Error::HasInstances`] while any of its environments has an instance on
    /// this machine, stopped or not: an instance's compose project, volumes and hostnames are
    /// named after the workspace, as is the mesh network, so they would be left behind under
    /// the old name.
    pub fn rename_workspace(&self, id: &WorkspaceId, name: &str) -> Result<()> {
        check_name(name)?;
        let old = self.workspace_by_id(id)?.name;
        if old == name {
            return Ok(());
        }
        self.refuse_workspace_instances(id, &old)?;
        self.shared.write(|sql| {
            let n = sql
                .execute(
                    "UPDATE workspace SET name = $2 WHERE id = $1",
                    args![&id.0, name],
                )
                .map_err(|e| duplicate_or(e, "workspace", name))?;
            found(n, "workspace", &id.0)
        })
    }

    fn refuse_workspace_instances(&self, id: &WorkspaceId, name: &str) -> Result<()> {
        let envs = self.environment_ids(id)?;
        refuse_instances(self.instances_of_envs(&envs)?, "workspace", name)
    }

    /// Set whether the workspace's gateways join a shared mesh network. Setting it to what it
    /// is does nothing.
    ///
    /// Refused with [`Error::HasInstances`] while any of its environments has an instance on
    /// this machine: an instance's gateway joins the mesh when it is created, so changing it
    /// under existing instances would leave them on (or off) a network the rest no longer
    /// share.
    pub fn set_mesh(&self, id: &WorkspaceId, mesh: bool) -> Result<()> {
        let ws = self.workspace_by_id(id)?;
        if ws.mesh == mesh {
            return Ok(());
        }
        self.refuse_workspace_instances(id, &ws.name)?;
        self.shared.write(|sql| {
            let n = sql.execute(
                "UPDATE workspace SET mesh = $2 WHERE id = $1",
                args![&id.0, mesh],
            )?;
            found(n, "workspace", &id.0)
        })
    }

    /// Replace the workspace's vars.
    pub fn set_workspace_vars(&self, id: &WorkspaceId, vars: &VarSet) -> Result<()> {
        self.shared.write(|sql| {
            let n = sql.execute(
                "UPDATE workspace SET vars = $2 WHERE id = $1",
                args![&id.0, json(vars)?],
            )?;
            found(n, "workspace", &id.0)
        })
    }

    /// Set the config every instance of the workspace binds the source `name` with, or clear it
    /// (`None`).
    pub fn set_workspace_source(
        &self,
        id: &WorkspaceId,
        name: &str,
        config: Option<serde_json::Value>,
    ) -> Result<()> {
        self.modify(
            "SELECT sources FROM workspace WHERE id = $1 FOR UPDATE",
            "UPDATE workspace SET sources = $2 WHERE id = $1",
            "workspace",
            &id.0,
            |text| {
                let mut sources: BTreeMap<String, serde_json::Value> = from_json(text)?;
                match config {
                    Some(config) => sources.insert(name.to_owned(), config),
                    None => sources.remove(name),
                };
                Ok::<_, Error>((json(&sources)?, ()))
            },
        )
    }

    /// Change the vars `scope` names with `f`, in one write transaction, so a concurrent writer's
    /// change to other vars of the same set is kept. Returns the vars as written.
    pub fn modify_vars<E: From<Error>>(
        &self,
        scope: &VarScope,
        f: impl FnOnce(&mut VarSet) -> std::result::Result<(), E>,
    ) -> std::result::Result<VarSet, E> {
        let (table, column, kind, id) = match scope {
            VarScope::Workspace(id) => ("workspace", "vars", "workspace", &id.0),
            VarScope::Environment(id) => ("environment", "vars", "environment", &id.0),
            VarScope::Draft(id) => ("environment", "draft_vars", "environment", &id.0),
            VarScope::Local(id) => return self.modify_local_vars(id, f),
        };
        self.modify(
            &format!("SELECT {column} FROM {table} WHERE id = $1 FOR UPDATE"),
            &format!("UPDATE {table} SET {column} = $2 WHERE id = $1"),
            kind,
            id,
            |text| {
                let mut vars: VarSet = from_json(text)?;
                f(&mut vars)?;
                Ok((json(&vars)?, vars))
            },
        )
    }

    /// This machine's own vars for the environment ([`VarScope::Local`]): none until set.
    pub fn local_vars(&self, env: &EnvironmentId) -> Result<VarSet> {
        self.local_read(|conn| {
            conn.query_row(
                "SELECT vars FROM local_var WHERE environment_id = ?1",
                [&env.0],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .map_or_else(|| Ok(VarSet::new()), |text| from_json(&text))
        })
    }

    fn modify_local_vars<E: From<Error>>(
        &self,
        env: &EnvironmentId,
        f: impl FnOnce(&mut VarSet) -> std::result::Result<(), E>,
    ) -> std::result::Result<VarSet, E> {
        // The environment may be held elsewhere; it must exist there.
        self.environment(env)?;
        let mut conn = lock(&self.local);
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(Error::from)?;
        let text: Option<String> = tx
            .query_row(
                "SELECT vars FROM local_var WHERE environment_id = ?1",
                [&env.0],
                |r| r.get(0),
            )
            .optional()
            .map_err(Error::from)?;
        let mut vars: VarSet = match text {
            Some(text) => from_json(&text)?,
            None => VarSet::new(),
        };
        f(&mut vars)?;
        tx.execute(
            "INSERT INTO local_var (environment_id, vars) VALUES (?1, ?2) \
             ON CONFLICT (environment_id) DO UPDATE SET vars = excluded.vars",
            params![env.0, json(&vars)?],
        )
        .map_err(Error::from)?;
        tx.commit().map_err(Error::from)?;
        Ok(vars)
    }

    /// Read the text `select` returns for `id` (locking its row), let `f` turn it into a new
    /// text and a result, and write the text with `update`: all in one write transaction.
    fn modify<T, E: From<Error>>(
        &self,
        select: &str,
        update: &str,
        kind: &'static str,
        id: &str,
        f: impl FnOnce(&str) -> std::result::Result<(String, T), E>,
    ) -> std::result::Result<T, E> {
        self.shared.write(|sql| {
            let text = sql
                .query_opt(select, args![id])?
                .ok_or_else(|| not_found(kind, id))?
                .text(0)?;
            let (text, out) = f(&text)?;
            sql.execute(update, args![id, text]).map_err(Error::from)?;
            Ok(out)
        })
    }

    /// Delete the workspace with all its environments and revisions, and this machine's
    /// instances of them with their leases.
    pub fn delete_workspace(&self, id: &WorkspaceId) -> Result<()> {
        let envs = self.environment_ids(id)?;
        self.shared.write(|sql| {
            let n = sql.execute("DELETE FROM workspace WHERE id = $1", args![&id.0])?;
            found(n, "workspace", &id.0)
        })?;
        self.forget_environments(&envs)
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
            draft_version: 0,
        };
        self.shared.write(|sql| {
            sql.execute(
                "INSERT INTO environment (id, workspace_id, name, draft, draft_modules, \
                 draft_vars, vars, head, draft_version, created_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, NULL, 0, $8)",
                args![
                    &env.id.0,
                    &workspace.0,
                    &env.name,
                    env.draft.manifest.to_toml()?,
                    json(&env.draft.modules)?,
                    json(&env.draft.vars)?,
                    json(&env.vars)?,
                    now()
                ],
            )
            .map_err(|e| match e {
                DbError::ForeignKey(_) => not_found("workspace", &workspace.0),
                e => duplicate_or(e, "environment", name),
            })?;
            Ok::<_, Error>(())
        })?;
        Ok(env)
    }

    /// The environments of `workspace`, by name.
    pub fn environments(&self, workspace: &WorkspaceId) -> Result<Vec<Environment>> {
        self.shared.read(|sql| {
            sql.query(
                &format!("{ENV_COLUMNS} WHERE workspace_id = $1 ORDER BY name"),
                args![&workspace.0],
            )?
            .iter()
            .map(environment_row)
            .collect()
        })
    }

    /// The environment `id`.
    pub fn environment(&self, id: &EnvironmentId) -> Result<Environment> {
        self.shared.read(|sql| environment_in(sql, id, false))
    }

    /// Rename the environment to `name`, within its workspace. Its id, revisions and vars stay
    /// as they are, and its draft's `env_name` follows the new name. The revisions keep the
    /// `env_name` they were committed with; an instance is always planned under the
    /// environment's current name. Renaming it to its own name does nothing.
    ///
    /// Refused with [`Error::HasInstances`] while it has an instance on this machine, stopped
    /// or not: an instance's compose project, volumes and hostnames are named after the
    /// environment, so they would be left behind under the old name.
    pub fn rename_environment(&self, id: &EnvironmentId, name: &str) -> Result<()> {
        check_name(name)?;
        let old = self.environment(id)?.name;
        if old == name {
            return Ok(());
        }
        refuse_instances(
            self.instances_of_envs(std::slice::from_ref(id))?,
            "environment",
            &old,
        )?;
        self.shared.write(|sql| {
            let env = environment_in(sql, id, true)?;
            let mut manifest = env.draft.manifest;
            manifest.context.env_name = name.to_owned();
            sql.execute(
                "UPDATE environment SET name = $2, draft = $3, \
                 draft_version = draft_version + 1 WHERE id = $1",
                args![&id.0, name, manifest.to_toml()?],
            )
            .map_err(|e| duplicate_or(e, "environment", name))?;
            Ok(())
        })
    }

    /// Replace the environment's draft, whatever was edited since it was read: an editor
    /// showing the draft to a user should use [`update_draft_if`](Self::update_draft_if).
    /// Its committed revisions are untouched. The draft's `env_name` is kept as the
    /// environment's name.
    pub fn update_draft(&self, id: &EnvironmentId, draft: &Draft) -> Result<()> {
        self.modify_draft(id, |d| {
            *d = draft.clone();
            Ok::<_, Error>(())
        })?;
        Ok(())
    }

    /// Replace the environment's draft if it is still at `expected`, the
    /// [`draft_version`](Environment::draft_version) read before editing it; returns the new
    /// version. Refused with [`Error::Conflict`] when someone edited it since (a colleague,
    /// on a shared store), so their edit isn't silently lost: reload and edit again.
    pub fn update_draft_if(&self, id: &EnvironmentId, draft: &Draft, expected: u64) -> Result<u64> {
        self.shared.write(|sql| {
            let env = environment_in(sql, id, true)?;
            if env.draft_version != expected {
                return Err(Error::Conflict {
                    name: env.name,
                    expected,
                    current: env.draft_version,
                });
            }
            let mut draft = draft.clone();
            draft.manifest.context.env_name = env.name;
            write_draft(sql, id, &draft)?;
            Ok(expected + 1)
        })
    }

    /// Change the environment's draft with `f`, in one write transaction, so two writers (the
    /// CLI and the app, say) never lose each other's edits. The draft's `env_name` is kept as
    /// the environment's name. Returns the draft as written.
    pub fn modify_draft<E: From<Error>>(
        &self,
        id: &EnvironmentId,
        f: impl FnOnce(&mut Draft) -> std::result::Result<(), E>,
    ) -> std::result::Result<Draft, E> {
        self.shared.write(|sql| {
            let env = environment_in(sql, id, true)?;
            let mut draft = env.draft;
            f(&mut draft)?;
            draft.manifest.context.env_name = env.name;
            write_draft(sql, id, &draft)?;
            Ok(draft)
        })
    }

    /// Replace the vars every revision of the environment sees.
    pub fn set_environment_vars(&self, id: &EnvironmentId, vars: &VarSet) -> Result<()> {
        self.shared.write(|sql| {
            let n = sql.execute(
                "UPDATE environment SET vars = $2 WHERE id = $1",
                args![&id.0, json(vars)?],
            )?;
            found(n, "environment", &id.0)
        })
    }

    /// Delete the environment with its revisions, and this machine's instances of it with
    /// their leases.
    pub fn delete_environment(&self, id: &EnvironmentId) -> Result<()> {
        self.shared.write(|sql| {
            let n = sql.execute("DELETE FROM environment WHERE id = $1", args![&id.0])?;
            found(n, "environment", &id.0)
        })?;
        self.forget_environments(std::slice::from_ref(id))
    }

    /// Commit the environment's draft as its next revision (numbered one past its head) and
    /// make it the head. The same as [`commit_draft_with`](Self::commit_draft_with) without a
    /// message.
    pub fn commit_draft(&self, id: &EnvironmentId) -> Result<Revision> {
        self.commit_draft_with(id, None)
    }

    /// Commit the environment's draft as its next revision, with `message` (trimmed; a blank
    /// one counts as none). The message isn't part of the revision's digest.
    pub fn commit_draft_with(&self, id: &EnvironmentId, message: Option<&str>) -> Result<Revision> {
        let message = message
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .map(str::to_owned);
        self.shared.write(|sql| {
            let row = sql
                .query_opt(
                    "SELECT draft, draft_modules, draft_vars, head FROM environment \
                     WHERE id = $1 FOR UPDATE",
                    args![&id.0],
                )?
                .ok_or_else(|| not_found("environment", &id.0))?;
            let draft = row.text(0)?;
            let modules: BTreeMap<String, String> = from_json(&row.text(1)?)?;
            let vars: VarSet = from_json(&row.text(2)?)?;
            let number = row.opt_u32(3)?.unwrap_or(0) + 1;
            let digest = digest(&draft, &modules, &vars)?;
            let created_at = now();
            sql.execute(
                "INSERT INTO revision (environment_id, number, manifest, modules, vars, digest, \
                 message, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                args![
                    &id.0,
                    number,
                    &draft,
                    json(&modules)?,
                    json(&vars)?,
                    &digest,
                    message.clone(),
                    created_at
                ],
            )?;
            sql.execute(
                "UPDATE environment SET head = $2 WHERE id = $1",
                args![&id.0, number],
            )?;
            Ok(Revision {
                environment: id.clone(),
                number,
                manifest: EnvManifest::from_toml(&draft)?,
                manifest_toml: draft,
                modules,
                vars,
                digest,
                message,
                created_at: time_of(created_at),
            })
        })
    }

    /// Revision `number` of the environment.
    pub fn revision(&self, env: &EnvironmentId, number: u32) -> Result<Revision> {
        self.shared.read(|sql| {
            sql.query_opt(
                &format!("{REV_COLUMNS} WHERE environment_id = $1 AND number = $2"),
                args![&env.0, number],
            )?
            .as_ref()
            .map(revision_row)
            .transpose()?
            .ok_or_else(|| not_found("revision", &format!("{env}#{number}")))
        })
    }

    /// Every revision of the environment, oldest first.
    pub fn revisions(&self, env: &EnvironmentId) -> Result<Vec<Revision>> {
        self.shared.read(|sql| {
            sql.query(
                &format!("{REV_COLUMNS} WHERE environment_id = $1 ORDER BY number"),
                args![&env.0],
            )?
            .iter()
            .map(revision_row)
            .collect()
        })
    }

    // --- Attachments -------------------------------------------------------------------------

    /// Record the shared store at `location` on this machine, as `name`. Nothing is
    /// connected to: [`open_attached`](Self::open_attached) does that.
    pub fn attach(&self, name: &str, location: RemoteLocation) -> Result<Attachment> {
        check_name(name)?;
        let attachment = Attachment {
            id: AttachmentId::generate(),
            name: name.to_owned(),
            location,
        };
        self.local_write(|tx| {
            tx.execute(
                "INSERT INTO attachment (id, name, location, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![
                    attachment.id.0,
                    attachment.name,
                    json(&attachment.location)?,
                    now()
                ],
            )
            .map_err(|e| {
                if !is_unique_violation(&e) {
                    return e.into();
                }
                let taken = tx
                    .query_row("SELECT 1 FROM attachment WHERE name = ?1", [name], |_| {
                        Ok(())
                    })
                    .optional()
                    .ok()
                    .flatten()
                    .is_some();
                Error::Duplicate {
                    kind: "attachment",
                    name: if taken {
                        name.to_owned()
                    } else {
                        attachment.location.to_string()
                    },
                }
            })?;
            Ok(())
        })?;
        Ok(attachment)
    }

    /// Every attachment on this machine, by name.
    pub fn attachments(&self) -> Result<Vec<Attachment>> {
        self.local_read(|conn| {
            let mut stmt =
                conn.prepare("SELECT id, name, location FROM attachment ORDER BY name")?;
            let rows = stmt.query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get(2)?))
            })?;
            rows.map(|r| {
                let (id, name, location): (String, String, String) = r?;
                Ok(Attachment {
                    id: AttachmentId(id),
                    name,
                    location: from_json(&location)?,
                })
            })
            .collect()
        })
    }

    /// The attachment `id`.
    pub fn attachment(&self, id: &AttachmentId) -> Result<Attachment> {
        self.local_read(|conn| attachment_row(conn, id))
    }

    /// Forget the attachment. The shared database is untouched. Refused with
    /// [`Error::HasInstances`] while this machine has an instance of one of its revisions.
    pub fn detach(&self, id: &AttachmentId) -> Result<()> {
        self.local_write(|tx| {
            let name: String = tx
                .query_row("SELECT name FROM attachment WHERE id = ?1", [&id.0], |r| {
                    r.get(0)
                })
                .optional()?
                .ok_or_else(|| not_found("attachment", &id.0))?;
            let instances: usize = tx.query_row(
                "SELECT COUNT(*) FROM instance WHERE store = ?1",
                [&id.0],
                |r| r.get(0),
            )?;
            refuse_instances(instances, "attachment", &name)?;
            tx.execute("DELETE FROM attachment WHERE id = ?1", [&id.0])?;
            Ok(())
        })
    }

    // --- Instances ---------------------------------------------------------------------------

    /// Create an instance of revision `revision` of the environment, stopped and without a
    /// slot.
    pub fn create_instance(&self, env: &EnvironmentId, revision: u32) -> Result<Instance> {
        self.revision(env, revision)?;
        let instance = Instance {
            id: InstanceId::generate(),
            store: self.attachment.clone(),
            environment: env.clone(),
            revision,
            desired: Desired::Stopped,
            overrides: VarSet::new(),
            sources: Vec::new(),
            slot: None,
        };
        self.local_write(|tx| {
            tx.execute(
                "INSERT INTO instance (id, store, environment_id, revision, desired, overrides, \
                 sources, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    instance.id.0,
                    instance.store.as_ref().map(|s| &s.0),
                    env.0,
                    revision,
                    instance.desired.as_str(),
                    json(&instance.overrides)?,
                    json(&instance.sources)?,
                    now()
                ],
            )?;
            Ok(())
        })?;
        Ok(instance)
    }

    /// The instance `id`.
    pub fn instance(&self, id: &InstanceId) -> Result<Instance> {
        self.local_read(|conn| {
            conn.query_row(
                &format!("{INST_COLUMNS} WHERE i.id = ?1"),
                [&id.0],
                instance_row,
            )
            .optional()?
            .transpose()?
            .ok_or_else(|| not_found("instance", &id.0))
        })
    }

    /// Every instance on the host, whichever store holds its environment, oldest first.
    pub fn instances(&self) -> Result<Vec<Instance>> {
        self.local_read(|conn| {
            let mut stmt = conn.prepare(&format!("{INST_COLUMNS} ORDER BY i.created_at, i.id"))?;
            let rows = stmt.query_map([], instance_row)?;
            rows.map(|r| r?).collect()
        })
    }

    /// The instances of the environment on this machine, oldest first.
    pub fn instances_of(&self, env: &EnvironmentId) -> Result<Vec<Instance>> {
        self.local_read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "{INST_COLUMNS} WHERE i.environment_id = ?1 ORDER BY i.created_at, i.id"
            ))?;
            let rows = stmt.query_map([&env.0], instance_row)?;
            rows.map(|r| r?).collect()
        })
    }

    /// Set whether the instance should be running.
    pub fn set_desired(&self, id: &InstanceId, desired: Desired) -> Result<()> {
        self.local_write(|tx| {
            let n = tx.execute(
                "UPDATE instance SET desired = ?2 WHERE id = ?1",
                params![id.0, desired.as_str()],
            )?;
            found(n as u64, "instance", &id.0)
        })
    }

    /// Replace the instance's overrides.
    pub fn set_overrides(&self, id: &InstanceId, overrides: &VarSet) -> Result<()> {
        self.local_write(|tx| {
            let n = tx.execute(
                "UPDATE instance SET overrides = ?2 WHERE id = ?1",
                params![id.0, json(overrides)?],
            )?;
            found(n as u64, "instance", &id.0)
        })
    }

    /// Replace the overlay sources the instance takes.
    pub fn set_sources(&self, id: &InstanceId, sources: &[SourceBinding]) -> Result<()> {
        self.local_write(|tx| {
            let n = tx.execute(
                "UPDATE instance SET sources = ?2 WHERE id = ?1",
                params![id.0, json(&sources)?],
            )?;
            found(n as u64, "instance", &id.0)
        })
    }

    /// Delete the instance, releasing its slot.
    pub fn delete_instance(&self, id: &InstanceId) -> Result<()> {
        self.local_write(|tx| {
            let n = tx.execute("DELETE FROM instance WHERE id = ?1", [&id.0])?;
            found(n as u64, "instance", &id.0)
        })
    }

    // --- Slot leases -------------------------------------------------------------------------

    /// Lease the instance the lowest slot no instance on the host holds, or return the one it
    /// already holds. Atomic across processes.
    pub fn lease_slot(&self, instance: &InstanceId) -> Result<SlotLease> {
        self.local_write(|tx| {
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
                    not_found("instance", &instance.0)
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
        self.local_write(|tx| {
            tx.execute(
                "DELETE FROM slot_lease WHERE instance_id = ?1",
                [&instance.0],
            )?;
            Ok(())
        })
    }

    /// Every lease on the host, by slot.
    pub fn leases(&self) -> Result<Vec<SlotLease>> {
        self.local_read(|conn| {
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

/// The store's path in the [state directory](trestle_home), creating the directory.
pub(crate) fn default_path() -> Result<PathBuf> {
    let home = trestle_home().ok_or_else(|| Error::NotFound {
        kind: "data directory",
        id: format!("(set {TRESTLE_HOME_VAR})"),
    })?;
    std::fs::create_dir_all(&home).map_err(|e| Error::io(&home, e))?;
    Ok(home.join(DB_FILE))
}

fn not_found(kind: &'static str, id: &str) -> Error {
    Error::NotFound {
        kind,
        id: id.to_owned(),
    }
}

fn found(rows: u64, kind: &'static str, id: &str) -> Result<()> {
    if rows == 0 {
        Err(not_found(kind, id))
    } else {
        Ok(())
    }
}

/// Bring `conn`'s schema up to [`SCHEMA_VERSION`], in one write transaction so two processes
/// opening a new store at once don't both migrate it. Foreign keys are off meanwhile, so a
/// migration can rebuild a table others reference (SQLite can't alter a table's
/// constraints); they're checked before it commits.
fn migrate(conn: &mut Connection) -> Result<()> {
    conn.pragma_update(None, "foreign_keys", false)?;
    let result = (|| {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let found: u32 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if found > SCHEMA_VERSION {
            return Err(Error::UnsupportedVersion {
                found,
                supported: SCHEMA_VERSION,
            });
        }
        if found == SCHEMA_VERSION {
            return Ok(());
        }
        for sql in &MIGRATIONS[found as usize..] {
            tx.execute_batch(sql)?;
        }
        let broken: Option<String> = tx
            .query_row("PRAGMA foreign_key_check", [], |r| r.get(0))
            .optional()?;
        if let Some(table) = broken {
            return Err(Error::Column(format!(
                "migrating left a dangling foreign key in `{table}`"
            )));
        }
        tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        tx.commit()?;
        Ok(())
    })();
    conn.pragma_update(None, "foreign_keys", true)?;
    result
}

fn attachment_row(conn: &Connection, id: &AttachmentId) -> Result<Attachment> {
    let (name, location): (String, String) = conn
        .query_row(
            "SELECT name, location FROM attachment WHERE id = ?1",
            [&id.0],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| not_found("attachment", &id.0))?;
    Ok(Attachment {
        id: id.clone(),
        name,
        location: from_json(&location)?,
    })
}

const WS_COLUMNS: &str = "SELECT id, name, mesh, vars, sources FROM workspace";

fn workspace_row(r: &Row) -> Result<Workspace> {
    Ok(Workspace {
        id: WorkspaceId(r.text(0)?),
        name: r.text(1)?,
        mesh: r.bool(2)?,
        vars: from_json(&r.text(3)?)?,
        sources: from_json(&r.text(4)?)?,
    })
}

const ENV_COLUMNS: &str = "SELECT id, workspace_id, name, draft, draft_modules, draft_vars, vars, \
     head, draft_version FROM environment";

fn environment_row(r: &Row) -> Result<Environment> {
    Ok(Environment {
        id: EnvironmentId(r.text(0)?),
        workspace: WorkspaceId(r.text(1)?),
        name: r.text(2)?,
        draft: Draft {
            manifest: EnvManifest::from_toml(&r.text(3)?)?,
            modules: from_json(&r.text(4)?)?,
            vars: from_json(&r.text(5)?)?,
        },
        vars: from_json(&r.text(6)?)?,
        head: r.opt_u32(7)?,
        draft_version: r.u64(8)?,
    })
}

/// The environment `id`, its row locked until the transaction ends when `lock`.
fn environment_in(sql: &mut dyn Sql, id: &EnvironmentId, lock: bool) -> Result<Environment> {
    let suffix = if lock { " FOR UPDATE" } else { "" };
    sql.query_opt(
        &format!("{ENV_COLUMNS} WHERE id = $1{suffix}"),
        args![&id.0],
    )?
    .as_ref()
    .map(environment_row)
    .transpose()?
    .ok_or_else(|| not_found("environment", &id.0))
}

/// Write `draft` as the environment's draft, bumping its version.
fn write_draft(sql: &mut dyn Sql, id: &EnvironmentId, draft: &Draft) -> Result<()> {
    sql.execute(
        "UPDATE environment SET draft = $2, draft_modules = $3, draft_vars = $4, \
         draft_version = draft_version + 1 WHERE id = $1",
        args![
            &id.0,
            draft.manifest.to_toml()?,
            json(&draft.modules)?,
            json(&draft.vars)?
        ],
    )?;
    Ok(())
}

const REV_COLUMNS: &str = "SELECT environment_id, number, manifest, modules, vars, digest, \
     message, created_at FROM revision";

fn revision_row(r: &Row) -> Result<Revision> {
    let manifest_toml = r.text(2)?;
    Ok(Revision {
        environment: EnvironmentId(r.text(0)?),
        number: r.u32(1)?,
        manifest: EnvManifest::from_toml(&manifest_toml)?,
        manifest_toml,
        modules: from_json(&r.text(3)?)?,
        vars: from_json(&r.text(4)?)?,
        digest: r.text(5)?,
        message: r.opt_text(6)?,
        created_at: time_of(r.int(7)?),
    })
}

const INST_COLUMNS: &str = "SELECT i.id, i.store, i.environment_id, i.revision, i.desired, \
     i.overrides, i.sources, l.slot FROM instance i LEFT JOIN slot_lease l ON l.instance_id = i.id";

fn instance_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Result<Instance>> {
    let (id, store, env, revision): (String, Option<String>, String, u32) =
        (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?);
    let (desired, overrides, sources, slot): (String, String, String, Option<u16>) =
        (r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?);
    Ok((|| {
        Ok(Instance {
            id: InstanceId(id),
            store: store.map(AttachmentId),
            environment: EnvironmentId(env),
            revision,
            desired: Desired::parse(&desired),
            overrides: from_json(&overrides)?,
            sources: from_json(&sources)?,
            slot,
        })
    })())
}
