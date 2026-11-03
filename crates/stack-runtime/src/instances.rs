//! [`Instances`]: running revisions from the [`Store`] as compose projects.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use olai_stack_state::{Desired, Instance, InstanceId, Revision, Store};
use olai_stack_topology::{
    Catalog, LOCAL_CREDENTIALS, LaunchEnvError, LocalCredentials, LocalSecrets, MaterializedOutput,
    Plan, PlanError, RenderError, RuntimePlacement, Secrets, Vantage, baseline_catalog_with,
};
use serde::Serialize;

use crate::effective::{EffectiveEnv, EnvLayers, Layer};
use crate::engine::{ComposeEngine, DownOptions, EngineStream, Progress, UpOptions};
use crate::events::{EngineEvent, EventAction, EventFilter};
use crate::ports::{BusyPort, probe_host_ports};
use crate::secrets::{SecretError, SecretProviders};
use crate::source::{Source, SourceCtx, SourceError};
use crate::status::Health;

/// The directory under the state directory instances are materialized into.
pub const INSTANCES_DIR: &str = "instances";

/// Keys the placement reserves in an instance's environment: no layer overrides them.
pub const PLACEMENT_KEYS: &[&str] = &[
    "COMPOSE_PROJECT_NAME",
    "TRESTLE_INSTANCE",
    "TRESTLE_SLOT",
    "TRESTLE_HOST_LABEL",
];

/// What can go wrong running an instance.
#[derive(Debug, thiserror::Error)]
pub enum InstanceError {
    /// The store failed.
    #[error(transparent)]
    Store(#[from] olai_stack_state::Error),
    /// The engine failed.
    #[error(transparent)]
    Engine(#[from] crate::Error),
    /// A module manifest the revision embeds doesn't parse.
    #[error("module manifest `{path}` of the revision: {reason}")]
    Module {
        /// The path the revision lists it under.
        path: String,
        /// The parse error.
        reason: String,
    },
    /// The revision doesn't plan.
    #[error(transparent)]
    Plan(#[from] PlanError),
    /// The plan doesn't render.
    #[error(transparent)]
    Render(#[from] RenderError),
    /// The rendered output would need a credential on disk.
    #[error(transparent)]
    LaunchEnv(#[from] LaunchEnvError),
    /// A secret couldn't be resolved.
    #[error(transparent)]
    Secret(#[from] SecretError),
    /// A source failed, or the instance names one that isn't configured.
    #[error(transparent)]
    Source(#[from] SourceError),
    /// A local credential the environment sets isn't valid.
    #[error("local credential: {0}")]
    LocalCredential(String),
    /// Host ports the instance would bind are taken.
    #[error("host ports are already in use: {}", ports(.0))]
    PortsBusy(Vec<BusyPort>),
    /// Another instance of the same revision is running (they would share a compose project).
    #[error("instance {running} already runs revision {revision} of this environment")]
    RevisionRunning {
        /// The revision.
        revision: u32,
        /// The instance running it.
        running: InstanceId,
    },
    /// Writing the materialized tree failed.
    #[error("{path}: {source}")]
    Io {
        /// The file or directory.
        path: PathBuf,
        /// The I/O error.
        #[source]
        source: std::io::Error,
    },
}

fn ports(busy: &[BusyPort]) -> String {
    busy.iter()
        .map(|b| format!("{} ({})", b.port, b.owner))
        .collect::<Vec<_>>()
        .join(", ")
}

type Result<T, E = InstanceError> = std::result::Result<T, E>;

/// What happened to one service of an instance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct InstanceEvent {
    /// The instance.
    pub instance: InstanceId,
    /// The compose service.
    pub service: String,
    /// What happened (see [`EventAction`]).
    pub action: EventAction,
}

/// What [`Instances::reconcile`] found.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Reconciled {
    /// Instances found running and marked as such.
    pub adopted: Vec<InstanceId>,
    /// Instances the store wanted running whose project is gone, now marked stopped.
    pub stale: Vec<InstanceId>,
    /// Instance-shaped projects no store instance accounts for.
    pub orphans: Vec<String>,
    /// Orphans that were taken down (with [`ReconcileOptions::remove_orphans`]).
    pub removed: Vec<String>,
}

/// How [`Instances::reconcile`] treats what it finds.
#[derive(Clone, Copy, Debug, Default)]
pub struct ReconcileOptions {
    /// Take down orphaned projects rather than only report them.
    pub remove_orphans: bool,
}

/// What [`Instances::materialize`] produced.
#[derive(Debug)]
pub struct Materialized {
    /// Where the tree was written.
    pub dir: PathBuf,
    /// The plan.
    pub plan: Plan,
    /// The resolved environment.
    pub env: EffectiveEnv,
    /// Everything Compose needs in its process environment: the effective environment plus
    /// the credentials moved out of the tree. Holds secrets; never print it.
    launch: BTreeMap<String, String>,
}

impl Materialized {
    /// The variable names the instance launches with (values withheld: they hold secrets).
    pub fn launch_var_names(&self) -> impl Iterator<Item = &str> {
        self.launch.keys().map(String::as_str)
    }
}

/// Runs the store's instances on an engine. Each instance runs one revision in its own compose
/// project, on its own slot and under its own host label, so two revisions of one environment
/// run side by side:
///
/// - project `<workspace>_<env>_r<revision>`: trestle names can't contain `_`, so a project
///   name parses back to its parts and [`reconcile`](Self::reconcile) tells instance projects
///   from any other;
/// - host label `<workspace>-<env>-r<revision>` (`demo-dev-r2.localhost`);
/// - the slot the store leases the instance, host-wide.
///
/// The rendered tree lives in `<state>/instances/<id>/` (owner-only) and is a cache,
/// re-rendered from the revision on every start. It holds no credential: they reach Compose as
/// process environment only (see [`MaterializedOutput::into_launch_env`]).
pub struct Instances<E> {
    store: Arc<Store>,
    engine: E,
    root: PathBuf,
    providers: SecretProviders,
    sources: BTreeMap<String, Arc<dyn Source>>,
    probe_ports: bool,
    wait_timeout: Option<Duration>,
}

/// The compose project of revision `revision` of `env` in `workspace`.
pub fn instance_project(workspace: &str, env: &str, revision: u32) -> String {
    format!("{workspace}_{env}_r{revision}")
}

/// `(workspace, env, revision)` of an instance project name, if it is one.
pub fn parse_instance_project(project: &str) -> Option<(&str, &str, u32)> {
    let mut parts = project.split('_');
    let (workspace, env, revision) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let revision = revision.strip_prefix('r')?.parse().ok()?;
    let valid = olai_stack_topology::workspace::is_valid_name;
    (valid(workspace) && valid(env)).then_some((workspace, env, revision))
}

impl<E: ComposeEngine> Instances<E> {
    /// Run `store`'s instances on `engine`, materializing them under `state_dir`
    /// (`<state_dir>/instances/<id>/`). Secrets resolve with the `env` provider only until
    /// [`with_providers`](Self::with_providers) says otherwise.
    pub fn new(store: Arc<Store>, engine: E, state_dir: impl Into<PathBuf>) -> Self {
        Instances {
            store,
            engine,
            root: state_dir.into().join(INSTANCES_DIR),
            providers: SecretProviders::new().with(crate::secrets::EnvProvider),
            sources: BTreeMap::new(),
            probe_ports: true,
            wait_timeout: None,
        }
    }

    /// Resolve secrets with `providers` (and no others).
    pub fn with_providers(mut self, providers: SecretProviders) -> Self {
        self.providers = providers;
        self
    }

    /// Make `source` available to instances that name it.
    pub fn with_source(mut self, source: impl Source + 'static) -> Self {
        self.sources
            .insert(source.name().to_owned(), Arc::new(source));
        self
    }

    /// Don't check host ports before starting (tests on a shared host, a fake engine).
    pub fn without_port_probe(mut self) -> Self {
        self.probe_ports = false;
        self
    }

    /// Give up waiting for health after `timeout` on start.
    pub fn with_wait_timeout(mut self, timeout: Duration) -> Self {
        self.wait_timeout = Some(timeout);
        self
    }

    /// The engine.
    pub fn engine(&self) -> &E {
        &self.engine
    }

    /// The store.
    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Where the instance's tree is materialized.
    pub fn dir(&self, id: &InstanceId) -> PathBuf {
        self.root.join(&id.0)
    }

    /// Where the instance runs: its compose project, host label, mesh and slot. Leases it a
    /// slot if it has none.
    pub fn placement(&self, id: &InstanceId) -> Result<RuntimePlacement> {
        let (instance, workspace, env) = self.describe(id)?;
        let slot = match instance.slot {
            Some(slot) => slot,
            None => self.store.lease_slot(id)?.slot,
        };
        Ok(RuntimePlacement {
            slot,
            project_name: instance_project(&workspace.name, &env.name, instance.revision),
            mesh_network: workspace
                .mesh
                .then(|| format!("trestle-{}-mesh", workspace.name)),
            host_label: format!("{}-{}-r{}", workspace.name, env.name, instance.revision),
        })
    }

    fn describe(
        &self,
        id: &InstanceId,
    ) -> Result<(
        Instance,
        olai_stack_state::Workspace,
        olai_stack_state::Environment,
    )> {
        let instance = self.store.instance(id)?;
        let env = self.store.environment(&instance.environment)?;
        let workspace = self.store.workspace_by_id(&env.workspace)?;
        Ok((instance, workspace, env))
    }

    /// The layers of the instance's environment that don't depend on its plan.
    async fn base_layers(
        &self,
        id: &InstanceId,
    ) -> Result<(EnvLayers, Revision, RuntimePlacement)> {
        let (instance, workspace, env) = self.describe(id)?;
        let revision = self
            .store
            .revision(&instance.environment, instance.revision)?;
        let placement = self.placement(id)?;
        let defaults: BTreeMap<String, String> = LOCAL_CREDENTIALS
            .iter()
            .map(|c| (c.env_var(), c.default.to_owned()))
            .collect();
        let reserved: BTreeMap<String, String> = [
            ("COMPOSE_PROJECT_NAME", placement.project_name.clone()),
            ("TRESTLE_INSTANCE", id.0.clone()),
            ("TRESTLE_SLOT", placement.slot.to_string()),
            ("TRESTLE_HOST_LABEL", placement.host_label.clone()),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
        let mut layers = EnvLayers::new()
            .with_secrets(Layer::CatalogDefaults, &defaults)
            .with_plain(Layer::Placement, &reserved)
            .with_vars(Layer::WorkspaceVars, &workspace.vars)
            .with_vars(Layer::EnvironmentVars, &env.vars)
            .with_vars(Layer::RevisionVars, revision.vars())
            .with_vars(Layer::InstanceOverrides, &instance.overrides);
        for binding in &instance.sources {
            let name = &binding.name;
            let source = self.sources.get(name).ok_or_else(|| SourceError {
                source_name: name.clone(),
                reason: "no such source is configured".into(),
            })?;
            let ctx = SourceCtx {
                instance: id.clone(),
                workspace: workspace.name.clone(),
                environment: env.name.clone(),
                revision: instance.revision,
                config: merge_config(workspace.sources.get(name), &binding.config),
            };
            let mut overlay = source.overlay(&ctx).await?;
            if overlay.is_expired(std::time::SystemTime::now()) {
                overlay = source.refresh(&ctx, &overlay).await?;
            }
            layers = layers.with_overlay(name, &overlay);
        }
        Ok((layers, revision, placement))
    }

    /// Plan the instance's revision in its placement under `env` (its secrets and local
    /// credentials come from there).
    fn plan(
        &self,
        revision: &Revision,
        env_name: &str,
        placement: &RuntimePlacement,
        env: &EffectiveEnv,
    ) -> Result<Plan> {
        let secrets: Secrets = env.launch_vars().into_iter().collect();
        let credentials = LocalCredentials::from_secrets(&secrets)
            .map_err(|e| InstanceError::LocalCredential(e.to_string()))?;
        let mut catalog = baseline_catalog_with(&credentials);
        for (path, text) in revision.modules() {
            let custom =
                Catalog::from_manifests([text.as_str()]).map_err(|e| InstanceError::Module {
                    path: path.clone(),
                    reason: e.to_string(),
                })?;
            catalog = catalog.merge(custom);
        }
        let mut ctx = revision.manifest().context.clone();
        ctx.env_name = env_name.to_owned();
        placement.apply(&mut ctx);
        // Credentials go to Compose as process env, never inline in a file.
        ctx.local_secrets = LocalSecrets::Separate;
        ctx.secrets = Some(secrets);
        Ok(catalog.plan(&revision.manifest().selection, &ctx)?)
    }

    /// The instance's environment, every variable with its provenance (see [`EffectiveEnv`]).
    pub async fn effective_env(&self, id: &InstanceId) -> Result<EffectiveEnv> {
        Ok(self.resolve(id).await?.0)
    }

    async fn resolve(&self, id: &InstanceId) -> Result<(EffectiveEnv, Plan)> {
        let (layers, revision, placement) = self.base_layers(id).await?;
        let env_name = self.store.environment(revision.environment())?.name;
        // The plan's own variables are layers too, but planning needs the credentials the
        // other layers resolve to: plan with those, then add the plan's layers and resolve
        // again.
        let first = layers.resolve(&self.providers)?;
        let plan = self.plan(&revision, &env_name, &placement, &first)?;
        // The plan's variables can carry a credential under a key of their own (a module's
        // password variable, say): those are secrets too.
        let secrets: Vec<&str> = first
            .vars()
            .values()
            .filter(|r| r.value.is_secret() && !r.value.expose().is_empty())
            .map(|r| r.value.expose())
            .collect();
        let split = |env: &olai_stack_topology::InjectedEnv| {
            let (secret, plain): (BTreeMap<_, _>, BTreeMap<_, _>) = env
                .iter()
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .partition(|(_, v)| secrets.iter().any(|s| v.contains(s)));
            (secret, plain)
        };
        let (injected_secret, injected) = split(&plan.env);
        let (client_secret, client) = split(plan.client_env(Vantage::Host));
        let env = layers
            .with_plain(Layer::RevisionManifest, &injected)
            .with_secrets(Layer::RevisionManifest, &injected_secret)
            .with_plain(Layer::ClientEnv, &client)
            .with_secrets(Layer::ClientEnv, &client_secret)
            .resolve(&self.providers)?;
        Ok((env, plan))
    }

    /// Render the instance into its directory, from scratch, and return what it launches
    /// with. The tree holds no credential.
    pub async fn materialize(&self, id: &InstanceId) -> Result<Materialized> {
        let (env, plan) = self.resolve(id).await?;
        let (output, launch_env) = plan.materialize()?.into_launch_env()?;
        let dir = self.dir(id);
        write_tree(&dir, &output)?;
        let mut launch = env.launch_vars();
        launch.extend(launch_env.into_vars());
        Ok(Materialized {
            dir,
            plan,
            env,
            launch,
        })
    }

    /// Start the instance: lease it a slot (if it has none), check its host ports, render it
    /// afresh and `up` it, waiting for health. The store records it as wanted running first,
    /// so a crash mid-start is reconciled as running.
    pub async fn start(&self, id: &InstanceId) -> Result<Materialized> {
        let instance = self.store.instance(id)?;
        let project = self.placement(id)?.project_name;
        for other in self.store.instances()? {
            if other.id != *id
                && other.environment == instance.environment
                && other.revision == instance.revision
                && other.desired == Desired::Running
            {
                return Err(InstanceError::RevisionRunning {
                    revision: instance.revision,
                    running: other.id,
                });
            }
        }
        let materialized = self.materialize(id).await?;
        if self.probe_ports {
            let running = self
                .engine
                .ls()
                .await?
                .into_iter()
                .any(|p| p.name == project && p.is_running());
            // A running project holds its own ports.
            if !running {
                let busy = probe_host_ports(&materialized.plan);
                if !busy.is_empty() {
                    return Err(InstanceError::PortsBusy(busy));
                }
            }
        }
        self.store.set_desired(id, Desired::Running)?;
        let opts = UpOptions {
            wait: true,
            wait_timeout: self.wait_timeout,
            env: materialized.launch.clone(),
            progress: Progress::Quiet,
        };
        self.engine.up(&materialized.dir, &opts).await?;
        Ok(materialized)
    }

    /// Stop the instance (`down`, with its volumes when `volumes`), keeping its slot.
    pub async fn stop(&self, id: &InstanceId, volumes: bool) -> Result<()> {
        // Compose interpolates the files on `down` too, so it needs the launch env again.
        let materialized = self.materialize(id).await?;
        let opts = DownOptions {
            volumes,
            env: materialized.launch,
            progress: Progress::Quiet,
        };
        self.engine.down(&materialized.dir, &opts).await?;
        self.store.set_desired(id, Desired::Stopped)?;
        Ok(())
    }

    /// Stop the instance if it runs, drop its tree, and delete it from the store (releasing
    /// its slot).
    pub async fn remove(&self, id: &InstanceId, volumes: bool) -> Result<()> {
        let project = self.placement(id)?.project_name;
        if self.engine.ls().await?.iter().any(|p| p.name == project) {
            self.stop(id, volumes).await?;
        }
        let dir = self.dir(id);
        if dir.exists() {
            std::fs::remove_dir_all(&dir)
                .map_err(|source| InstanceError::Io { path: dir, source })?;
        }
        self.store.delete_instance(id)?;
        Ok(())
    }

    /// The health of each of the instance's services (empty when it doesn't run).
    pub async fn health(&self, id: &InstanceId) -> Result<BTreeMap<String, Health>> {
        let project = self.placement(id)?.project_name;
        Ok(self
            .engine
            .ps(&project)
            .await?
            .into_iter()
            .map(|s| (s.service, s.health))
            .collect())
    }

    /// The instance's events from now on: its containers' lifecycle and health changes.
    pub async fn events(&self, id: &InstanceId) -> Result<EngineStream<InstanceEvent>> {
        let project = self.placement(id)?.project_name;
        let id = id.clone();
        Ok(self
            .engine
            .events(&EventFilter::Project(project))
            .await?
            .map(move |e: crate::Result<EngineEvent>| {
                e.map(|e| InstanceEvent {
                    instance: id.clone(),
                    service: e.service,
                    action: e.action,
                })
            })
            .boxed())
    }

    /// Match what Docker runs against the store, after a restart of the runtime: a running
    /// instance project is adopted (its instance marked running), an instance wanted running
    /// whose project is gone is marked stopped, and an instance-shaped project the store
    /// doesn't know is an orphan (taken down with [`ReconcileOptions::remove_orphans`]).
    pub async fn reconcile(&self, opts: ReconcileOptions) -> Result<Reconciled> {
        let projects = self.engine.ls().await?;
        let mut by_project: BTreeMap<String, InstanceId> = BTreeMap::new();
        let instances = self.store.instances()?;
        for instance in &instances {
            let env = self.store.environment(&instance.environment)?;
            let ws = self.store.workspace_by_id(&env.workspace)?;
            by_project.insert(
                instance_project(&ws.name, &env.name, instance.revision),
                instance.id.clone(),
            );
        }
        let mut out = Reconciled::default();
        let mut present = BTreeSet::new();
        for project in &projects {
            if parse_instance_project(&project.name).is_none() {
                continue;
            }
            present.insert(project.name.clone());
            match by_project.get(&project.name) {
                Some(id) if project.is_running() => {
                    // Several instances of one revision share a project name; the first
                    // wanted running (or the first) is the one that runs it.
                    self.store.set_desired(id, Desired::Running)?;
                    out.adopted.push(id.clone());
                }
                Some(_) => {}
                None => {
                    out.orphans.push(project.name.clone());
                    if opts.remove_orphans && self.remove_orphan(&project.config_files).await? {
                        out.removed.push(project.name.clone());
                    }
                }
            }
        }
        for instance in instances {
            if instance.desired != Desired::Running {
                continue;
            }
            let project = by_project
                .iter()
                .find(|(_, id)| **id == instance.id)
                .map(|(p, _)| p.clone())
                .unwrap_or_default();
            if !present.contains(&project) {
                self.store.set_desired(&instance.id, Desired::Stopped)?;
                out.stale.push(instance.id);
            }
        }
        Ok(out)
    }

    /// Take down the project started from `config_files`; `false` when Compose didn't say
    /// where that was (or it is gone), so there is nothing to run `down` in.
    async fn remove_orphan(&self, config_files: &str) -> Result<bool> {
        // Compose lists the files it started the project from; `down` needs their directory.
        let Some(dir) = config_files
            .split(',')
            .next()
            .filter(|f| !f.is_empty())
            .map(Path::new)
            .and_then(Path::parent)
            .filter(|d| d.is_dir())
        else {
            return Ok(false);
        };
        let opts = DownOptions {
            progress: Progress::Quiet,
            ..DownOptions::default()
        };
        self.engine.down(dir, &opts).await?;
        Ok(true)
    }
}

/// A source's config for one instance: the binding's keys over the workspace's (objects merge
/// one level deep; anything else replaces).
fn merge_config(
    workspace: Option<&serde_json::Value>,
    binding: &serde_json::Value,
) -> serde_json::Value {
    use serde_json::Value;
    match (workspace, binding) {
        (None, b) => b.clone(),
        (Some(w), Value::Null) => w.clone(),
        (Some(Value::Object(w)), Value::Object(b)) => {
            let mut merged = w.clone();
            merged.extend(b.iter().map(|(k, v)| (k.clone(), v.clone())));
            Value::Object(merged)
        }
        (Some(_), b) => b.clone(),
    }
}

/// Write `output` into `dir`, replacing whatever was there, with the directory owner-only.
fn write_tree(dir: &Path, output: &MaterializedOutput) -> Result<()> {
    let io = |path: &Path| {
        let path = path.to_owned();
        move |source| InstanceError::Io { path, source }
    };
    if dir.exists() {
        std::fs::remove_dir_all(dir).map_err(io(dir))?;
    }
    std::fs::create_dir_all(dir).map_err(io(dir))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(io(dir))?;
    }
    output.write_to(dir).map_err(io(dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_projects_parse_back() {
        let name = instance_project("demo-x", "prod", 3);
        assert_eq!(name, "demo-x_prod_r3");
        assert_eq!(parse_instance_project(&name), Some(("demo-x", "prod", 3)));
        for other in ["demo-dev", "a_b", "a_b_c", "a_b_r", "A_b_r1", "a_b_r1_x"] {
            assert_eq!(parse_instance_project(other), None, "{other}");
        }
    }
}
