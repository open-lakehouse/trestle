//! What the store-backed commands (`ws`, `env create …`, `var`, `up`/`down`/`ps`/`logs`/`health`)
//! share: the state store under `$TRESTLE_HOME`, the Docker engine, and where secret values go.

#[cfg(feature = "test-engine")]
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use olai_stack_runtime::neon::{NeonBranchSource, NeonCli, NeonError};
use olai_stack_runtime::{
    ComposeEngine, DockerCli, DownOptions, EngineEvent, EngineStream, EnvProvider, EventFilter,
    Instances, LogLine, LogOptions, ProjectStatus, SecretError, SecretProvider, SecretProviders,
    SecretValue, ServiceStatus, UpOptions, instance_project,
};
use olai_stack_state::{
    DB_FILE, Environment, Instance, SecretRef, Store, TRESTLE_HOME_VAR, VarSet, VarValue,
    Workspace, trestle_home,
};

use crate::error::{Error, Result};

/// The keychain service trestle keeps secret values under.
pub(super) const KEYCHAIN_SERVICE: &str = "trestle";

/// The provider name secret references to the keychain carry.
pub(super) const KEYCHAIN: &str = "keychain";

/// Test seam (feature `test-engine`): run on a [`FakeEngine`](olai_stack_runtime::FakeEngine)
/// whose running projects persist in this JSON file between invocations.
#[cfg(feature = "test-engine")]
const FAKE_ENGINE_VAR: &str = "TRESTLE_FAKE_ENGINE";

/// Test seam (feature `test-engine`): keep "keychain" secrets in this JSON file.
#[cfg(feature = "test-engine")]
const FAKE_KEYCHAIN_VAR: &str = "TRESTLE_FAKE_KEYCHAIN";

/// The state the store-backed commands work on.
pub(super) struct Ctx {
    pub store: Arc<Store>,
    home: PathBuf,
    engine: CliEngine,
    secrets: SecretStore,
}

impl Ctx {
    /// Open the store in the trestle home, with the Docker engine and the OS keychain.
    pub fn open() -> Result<Self> {
        let home = trestle_home().ok_or_else(|| {
            Error::other("no data directory for the trestle state")
                .with_hint(format!("set {TRESTLE_HOME_VAR}"))
        })?;
        std::fs::create_dir_all(&home).map_err(|e| Error::io_at(&home, e))?;
        let store = Arc::new(Store::open(&home.join(DB_FILE))?);
        Ok(Ctx {
            store,
            home,
            engine: CliEngine::from_env()?,
            secrets: SecretStore::from_env(),
        })
    }

    /// The engine.
    pub fn engine(&self) -> &CliEngine {
        &self.engine
    }

    /// The store's instances, run on the engine, with secrets from the process environment and
    /// the keychain.
    pub fn instances(&self) -> Instances<CliEngine> {
        let providers = SecretProviders::new()
            .with(EnvProvider)
            .with(self.secrets.clone());
        let mut instances =
            Instances::new(Arc::clone(&self.store), self.engine.clone(), &self.home)
                .with_providers(providers.clone());
        // Instances bound to a Neon branch take its overlay. Without the Neon CLI nothing can
        // be bound, and a pure compose environment never needs it.
        if let Ok(neon) = NeonCli::new() {
            instances = instances.with_source(NeonBranchSource::new(Arc::new(neon), providers));
        }
        if self.engine.is_fake() {
            instances.without_port_probe()
        } else {
            instances
        }
    }

    /// The Neon CLI, or an error saying how to get it.
    pub fn neon(&self) -> Result<NeonCli> {
        NeonCli::new().map_err(neon_err)
    }

    /// Where secret values go.
    pub fn secrets(&self) -> &SecretStore {
        &self.secrets
    }

    /// Run `fut` to completion on a single-threaded runtime, then save what a fake engine
    /// holds.
    pub fn block_on<T>(&self, fut: impl std::future::Future<Output = Result<T>>) -> Result<T> {
        let out = block_on(fut);
        self.engine.save()?;
        out
    }
}

/// `e` with a hint at what to do about it.
pub(super) fn neon_err(e: NeonError) -> Error {
    let hint = match &e {
        NeonError::NotInstalled => Some(
            "install the Neon CLI (`brew install neonctl` or `npm i -g neonctl`), or point \
             NEON_CLI at it",
        ),
        NeonError::NotAuthenticated(_) => Some("`neon auth`, or set NEON_API_KEY"),
        NeonError::MissingFunctions { .. } => {
            Some("`neon functions deploy <slug> --branch <branch>`, or bind a branch made since")
        }
        _ => None,
    };
    match hint {
        Some(h) => Error::from(e).with_hint(h),
        None => e.into(),
    }
}

/// Drive `fut` to completion on a single-threaded runtime.
pub(super) fn block_on<T>(fut: impl std::future::Future<Output = Result<T>>) -> Result<T> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| Error::other(format!("starting the async runtime: {e}")))?
        .block_on(fut)
}

/// The engine the CLI drives: Docker, or (feature `test-engine`) a fake one.
#[derive(Clone)]
pub(super) enum CliEngine {
    Docker(DockerCli),
    #[cfg(feature = "test-engine")]
    Fake(olai_stack_runtime::FakeEngine, PathBuf),
}

impl CliEngine {
    fn from_env() -> Result<Self> {
        #[cfg(feature = "test-engine")]
        if let Some(path) = std::env::var_os(FAKE_ENGINE_VAR) {
            let path = PathBuf::from(path);
            let engine = olai_stack_runtime::FakeEngine::new();
            if path.exists() {
                let text = std::fs::read_to_string(&path).map_err(|e| Error::io_at(&path, e))?;
                let projects: Vec<String> = serde_json::from_str(&text)
                    .map_err(|e| Error::other(format!("{}: {e}", path.display())))?;
                for project in projects {
                    engine.adopt(&project);
                }
            }
            return Ok(CliEngine::Fake(engine, path));
        }
        Ok(CliEngine::Docker(DockerCli::new()))
    }

    fn is_fake(&self) -> bool {
        !matches!(self, CliEngine::Docker(_))
    }

    fn save(&self) -> Result<()> {
        #[cfg(feature = "test-engine")]
        if let CliEngine::Fake(engine, path) = self {
            let projects: Vec<String> = engine.projects().into_iter().collect();
            let text = serde_json::to_string(&projects).map_err(|e| Error::other(e.to_string()))?;
            std::fs::write(path, text).map_err(|e| Error::io_at(path, e))?;
        }
        Ok(())
    }

    fn inner(&self) -> &dyn ComposeEngine {
        match self {
            CliEngine::Docker(d) => d,
            #[cfg(feature = "test-engine")]
            CliEngine::Fake(f, _) => f,
        }
    }
}

#[async_trait]
impl ComposeEngine for CliEngine {
    async fn up(&self, dir: &Path, opts: &UpOptions) -> olai_stack_runtime::Result<()> {
        self.inner().up(dir, opts).await
    }

    async fn down(&self, dir: &Path, opts: &DownOptions) -> olai_stack_runtime::Result<()> {
        self.inner().down(dir, opts).await
    }

    async fn ls(&self) -> olai_stack_runtime::Result<Vec<ProjectStatus>> {
        self.inner().ls().await
    }

    async fn ps(&self, project: &str) -> olai_stack_runtime::Result<Vec<ServiceStatus>> {
        self.inner().ps(project).await
    }

    async fn logs(
        &self,
        project: &str,
        opts: &LogOptions,
    ) -> olai_stack_runtime::Result<EngineStream<LogLine>> {
        self.inner().logs(project, opts).await
    }

    async fn events(
        &self,
        filter: &EventFilter,
    ) -> olai_stack_runtime::Result<EngineStream<EngineEvent>> {
        self.inner().events(filter).await
    }

    async fn network_rm(&self, name: &str) -> olai_stack_runtime::Result<bool> {
        self.inner().network_rm(name).await
    }
}

/// Where `var set --secret` puts values: the OS keychain, or (feature `test-engine`) a file.
/// Resolves the references as the `keychain` provider.
#[derive(Clone)]
pub(super) enum SecretStore {
    Keychain(olai_stack_runtime::KeychainProvider),
    #[cfg(feature = "test-engine")]
    File(PathBuf),
}

impl SecretStore {
    fn from_env() -> Self {
        #[cfg(feature = "test-engine")]
        if let Some(path) = std::env::var_os(FAKE_KEYCHAIN_VAR) {
            return SecretStore::File(PathBuf::from(path));
        }
        SecretStore::Keychain(olai_stack_runtime::KeychainProvider::new(KEYCHAIN_SERVICE))
    }

    /// Store `value` under `key`.
    pub fn set(&self, key: &str, value: &SecretValue) -> Result<()> {
        match self {
            SecretStore::Keychain(k) => k.set(key, value).map_err(secret_err),
            #[cfg(feature = "test-engine")]
            SecretStore::File(path) => {
                let mut map = read_file_secrets(path)?;
                map.insert(key.to_owned(), value.expose().to_owned());
                write_file_secrets(path, &map)
            }
        }
    }

    /// Delete the value under `key`, if there is one.
    pub fn delete(&self, key: &str) -> Result<()> {
        match self {
            SecretStore::Keychain(k) => match k.delete(key) {
                Ok(()) | Err(SecretError::Missing { .. }) => Ok(()),
                Err(e) => Err(secret_err(e)),
            },
            #[cfg(feature = "test-engine")]
            SecretStore::File(path) => {
                let mut map = read_file_secrets(path)?;
                map.remove(key);
                write_file_secrets(path, &map)
            }
        }
    }
}

impl SecretProvider for SecretStore {
    fn name(&self) -> &str {
        KEYCHAIN
    }

    fn get(&self, key: &str) -> std::result::Result<SecretValue, SecretError> {
        match self {
            SecretStore::Keychain(k) => k.get(key),
            #[cfg(feature = "test-engine")]
            SecretStore::File(path) => read_file_secrets(path)
                .ok()
                .and_then(|m| m.get(key).cloned())
                .map(SecretValue::new)
                .ok_or_else(|| SecretError::Missing {
                    provider: KEYCHAIN.into(),
                    key: key.into(),
                }),
        }
    }
}

#[cfg(feature = "test-engine")]
fn read_file_secrets(path: &Path) -> Result<BTreeMap<String, String>> {
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let text = std::fs::read_to_string(path).map_err(|e| Error::io_at(path, e))?;
    serde_json::from_str(&text).map_err(|e| Error::other(format!("{}: {e}", path.display())))
}

#[cfg(feature = "test-engine")]
fn write_file_secrets(path: &Path, map: &BTreeMap<String, String>) -> Result<()> {
    let text = serde_json::to_string(map).map_err(|e| Error::other(e.to_string()))?;
    std::fs::write(path, text).map_err(|e| Error::io_at(path, e))
}

fn secret_err(e: SecretError) -> Error {
    Error::other(format!("keychain: {e}"))
}

/// Every keychain reference `vars` holds.
pub(super) fn keychain_refs(vars: &VarSet) -> impl Iterator<Item = &SecretRef> {
    vars.0.values().filter_map(|v| match v {
        VarValue::Secret(r) if r.provider == KEYCHAIN => Some(r),
        _ => None,
    })
}

/// Every keychain reference the workspace and its environments (drafts and revisions) hold.
pub(super) fn workspace_keychain_refs(store: &Store, ws: &Workspace) -> Result<Vec<SecretRef>> {
    let mut refs: Vec<SecretRef> = keychain_refs(&ws.vars).cloned().collect();
    for env in store.environments(&ws.id)? {
        refs.extend(environment_keychain_refs(store, &env)?);
    }
    Ok(refs)
}

/// Every keychain reference the environment (its vars, draft and revisions) holds.
pub(super) fn environment_keychain_refs(
    store: &Store,
    env: &Environment,
) -> Result<Vec<SecretRef>> {
    let mut refs: Vec<SecretRef> = keychain_refs(&env.vars)
        .chain(keychain_refs(&env.draft.vars))
        .cloned()
        .collect();
    for rev in store.revisions(&env.id)? {
        refs.extend(keychain_refs(rev.vars()).cloned());
    }
    refs.sort();
    refs.dedup();
    Ok(refs)
}

/// The compose project the instance runs as (computed, so no slot is leased for it).
pub(super) fn project_of(store: &Store, instance: &Instance) -> Result<String> {
    let env = store.environment(&instance.environment)?;
    let ws = store.workspace_by_id(&env.workspace)?;
    Ok(instance_project(&ws.name, &env.name, instance.revision))
}

/// What Docker runs, or nothing when Docker isn't there (nothing runs then).
pub(super) async fn running_projects(ctx: &Ctx) -> Result<Vec<ProjectStatus>> {
    match ctx.engine().ls().await {
        Ok(p) => Ok(p),
        Err(e) if e.is_unavailable() => Ok(Vec::new()),
        Err(e) => Err(e.into()),
    }
}

/// The first of `instances` that Docker runs or the store wants running.
pub(super) async fn any_running(ctx: &Ctx, instances: &[Instance]) -> Result<Option<Instance>> {
    let projects = running_projects(ctx).await?;
    for instance in instances {
        let project = project_of(&ctx.store, instance)?;
        let running = projects.iter().any(|p| p.name == project && p.is_running());
        if running || instance.desired == olai_stack_state::Desired::Running {
            return Ok(Some(instance.clone()));
        }
    }
    Ok(None)
}
