//! The persisted, deterministic record of a user's environment choices: [`EnvManifest`].
//!
//! An environment is fully determined by two inputs to [`Catalog::plan`](crate::Catalog::plan):
//! a [`Selection`] (which modules + knob overrides) and a [`PlanCtx`] (the environment-level
//! facts the model can't derive — gateway ports, `data_root`, provider preferences, the app
//! upstream). An [`EnvManifest`] bundles both and (de)serializes to **TOML**, so a consumer can
//! save a user's choices and re-create the *same* environment later.
//!
//! # Stability across edits
//!
//! The manifest is also the **edit surface**: load it, [`add_module`](EnvManifest::add_module) /
//! [`remove_module`](EnvManifest::remove_module), and re-[`plan`](EnvManifest::plan). The
//! **shared gateway listener is stable by construction** — its host port comes from
//! [`PlanCtx::gateway_host_port`] (persisted here, never re-derived), and every shared route's
//! prefix / rewrite / cluster is derived purely from the owning module's own static data, so a
//! surviving module keeps its exact mapping no matter what else is added or removed. This is the
//! guarantee host applications depend on: the gateway port and the route to a given service do
//! not move when the selection is edited.
//!
//! Two things *may* move on an edit, by design:
//!
//! - **Dedicated/secondary listener ports** ([`UiFixed`](crate::RouteIntent::UiFixed) /
//!   [`Gatewayed`](crate::RouteIntent::Gatewayed)) — allocated sequentially in graph order, so
//!   adding a module with such an endpoint can shift later ones. Use
//!   [`pin_dedicated_ports`] to hold them stable best-effort (it feeds
//!   [`PlanCtx::dedicated_listener_ports`] on the next plan).
//! - **Provider selection** for an *unpinned* resource demand — set
//!   [`PlanCtx::provider_preference`] (persisted here) to pin which implementation satisfies a
//!   role, and this is stable too.
//!
//! # Purity
//!
//! [`to_toml`](EnvManifest::to_toml) / [`from_toml`](EnvManifest::from_toml) are pure and
//! WASM-clean (they produce / consume a `String`). Only [`read_from`](EnvManifest::read_from) /
//! [`write_to`](EnvManifest::write_to) touch the filesystem, and they sit behind the `std-io`
//! feature — mirroring [`MaterializedOutput::write_to`](crate::MaterializedOutput::write_to).

use serde::{Deserialize, Serialize};

use crate::catalog::Catalog;
use crate::catalog::module::{ExtraResource, ModuleId};
use crate::model::connection::ObjectStoreCredential;
use crate::plan::{HostedConnection, HostedResource, Plan, PlanCtx, PlanError, Selection};

/// What can go wrong (de)serializing an [`EnvManifest`].
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    /// Serializing the manifest to TOML failed.
    #[error("serializing the environment manifest to TOML failed: {0}")]
    Serialize(#[from] toml::ser::Error),
    /// Parsing the manifest from TOML failed (malformed file or unknown shape).
    #[error("parsing the environment manifest from TOML failed: {0}")]
    Deserialize(#[from] toml::de::Error),
    /// The manifest was written by a newer version than this one understands.
    #[error(
        "environment manifest version {found} is newer than this version supports ({supported}); \
         upgrade trestle"
    )]
    UnsupportedVersion {
        /// The file's `version`.
        found: u32,
        /// The newest version this crate reads.
        supported: u32,
    },
    /// A version-2 `[selection.external.<module>]` entry can't be migrated to a hosted
    /// provider (see [`EnvManifest::from_toml`]).
    #[error("cannot migrate external resource `{module}` to a hosted provider: {reason}")]
    LegacyExternal {
        /// The module the legacy entry marked external.
        module: String,
        /// Why it can't be migrated.
        reason: String,
    },
    /// Reading or writing the manifest file failed (only the `std-io` helpers).
    #[cfg(feature = "std-io")]
    #[error("reading/writing the environment manifest file failed: {0}")]
    Io(#[from] std::io::Error),
}

/// A persisted, deterministic record of a user's environment choices: the module + knob
/// [`Selection`] and the environment-level [`PlanCtx`]. Re-[`plan`](Self::plan)ning from the same
/// manifest yields the same [`Plan`] — in particular the same gateway host port, which host
/// applications depend on. See the [module docs](crate::manifest) for the stability contract.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvManifest {
    /// The manifest schema version, so older files stay readable as the shape evolves.
    /// Omitted files default to [`current_version`](Self::current_version).
    #[serde(default = "EnvManifest::current_version")]
    pub version: u32,
    /// Which modules / capabilities are selected and their knob overrides.
    #[serde(default)]
    pub selection: Selection,
    /// The environment-level plan context — gateway ports, `data_root`, provider preferences,
    /// app upstream. A persisted non-default `gateway_host_port` is authoritative on reload, so
    /// the host-facing port stays put across re-plans.
    #[serde(default)]
    pub context: PlanCtx,
}

impl EnvManifest {
    /// The current manifest schema version. Version 2 added
    /// [`Selection::gateway`](crate::Selection::gateway); a version-1 file still reads (the
    /// planner honours its legacy gateway forms, see
    /// [`normalize_gateway`](Self::normalize_gateway)). Version 3 replaced
    /// `[selection.external.<module>]` (an in-stack provider marked external) with hosted
    /// provider modules ([`Selection::hosted`](crate::Selection::hosted)); an older file's
    /// entries are migrated on read (see [`from_toml`](Self::from_toml)).
    pub fn current_version() -> u32 {
        3
    }

    /// A manifest for `selection` under `context`, at the current schema version.
    pub fn new(selection: Selection, context: PlanCtx) -> Self {
        EnvManifest {
            version: Self::current_version(),
            selection,
            context,
        }
    }

    /// Serialize this manifest to a TOML string. Pure and WASM-clean.
    pub fn to_toml(&self) -> Result<String, ManifestError> {
        Ok(toml::to_string_pretty(self)?)
    }

    /// Parse a manifest from a TOML string. Pure and WASM-clean. Fields the file omits fall back
    /// to their defaults (e.g. an absent `[context]` uses [`PlanCtx::default`]).
    ///
    /// Errors with [`ManifestError::UnsupportedVersion`] for a file newer than this crate: it
    /// may carry settings this version would silently drop (such as gateway sign-in).
    ///
    /// A pre-version-3 file's `[selection.external.<module>]` entries are migrated to the hosted
    /// provider of their connection's kind — `hosted-postgres`, `aws-s3` or `azure-storage` —
    /// which replaces the in-stack module in the selection (its knob overrides are dropped), and
    /// the manifest is stamped with the current version. An Azure entry whose storage account
    /// can't be read off its endpoint is a [`ManifestError::LegacyExternal`].
    pub fn from_toml(s: &str) -> Result<Self, ManifestError> {
        let mut table: toml::Table = toml::from_str(s)?;
        let migrated = migrate_external(&mut table)?;
        let mut manifest: Self = toml::Value::Table(table).try_into()?;
        if manifest.version > Self::current_version() {
            return Err(ManifestError::UnsupportedVersion {
                found: manifest.version,
                supported: Self::current_version(),
            });
        }
        if migrated {
            manifest.version = Self::current_version();
        }
        Ok(manifest)
    }

    /// Rewrite the selection's legacy gateway forms into
    /// [`Selection::gateway`](crate::Selection::gateway) and stamp the current version; see
    /// [`Selection::normalize_gateway`]. Plans the same before and after.
    pub fn normalize_gateway(&mut self, catalog: &Catalog) -> Result<(), PlanError> {
        self.selection.normalize_gateway(catalog, &self.context)?;
        self.version = Self::current_version();
        Ok(())
    }

    /// Resolve this manifest into a [`Plan`] — the single "manifest → environment" entry point.
    /// Wraps [`Catalog::plan`](crate::Catalog::plan) with the manifest's own selection + context.
    pub fn plan(&self, catalog: &Catalog) -> Result<Plan, PlanError> {
        catalog.plan(&self.selection, &self.context)
    }

    /// Select `id`, if not already selected. Idempotent.
    pub fn add_module(&mut self, id: impl Into<ModuleId>) {
        let id = id.into();
        if !self.selection.modules.contains(&id) {
            self.selection.modules.push(id);
        }
    }

    /// Deselect `id` and drop any knob overrides recorded for it, keeping the manifest tidy.
    pub fn remove_module(&mut self, id: &ModuleId) {
        self.selection.modules.retain(|m| m != id);
        self.selection.knob_overrides.remove(id);
    }

    /// Select capability `cap`, if not already selected. Idempotent.
    pub fn add_capability(&mut self, cap: impl Into<String>) {
        let cap = cap.into();
        if !self.selection.capabilities.contains(&cap) {
            self.selection.capabilities.push(cap);
        }
    }

    /// Deselect capability `cap`.
    pub fn remove_capability(&mut self, cap: &str) {
        self.selection.capabilities.retain(|c| c != cap);
    }

    /// Declare an environment-level [`ExtraResource`] to provision, if not already declared
    /// (matched on `(resource, name)`). Idempotent; a later call with the same role+name but a
    /// different provider pin is a no-op — [`remove_extra_resource`](Self::remove_extra_resource)
    /// first to change a pin.
    pub fn add_extra_resource(&mut self, extra: ExtraResource) {
        if !self
            .selection
            .extra_resources
            .iter()
            .any(|e| e.resource == extra.resource && e.name == extra.name)
        {
            self.selection.extra_resources.push(extra);
        }
    }

    /// Drop the environment-level extra resource matching `(resource, name)`, if present.
    pub fn remove_extra_resource(&mut self, resource: &str, name: &str) {
        self.selection
            .extra_resources
            .retain(|e| !(e.resource == resource && e.name == name));
    }

    /// Set a knob override for `module`: `key` → `value`, replacing any previous value.
    pub fn set_knob(
        &mut self,
        module: impl Into<ModuleId>,
        key: impl Into<String>,
        value: impl Into<String>,
    ) {
        self.selection
            .knob_overrides
            .entry(module.into())
            .or_default()
            .insert(key.into(), value.into());
    }
}

/// Capture the dedicated-listener host ports a plan assigned, as the explicit
/// [`dedicated_listener_ports`](PlanCtx::dedicated_listener_ports) list to feed a later re-plan so
/// surviving dedicated endpoints keep their ports.
///
/// The ports are returned in the plan's listener order (graph order), so re-feeding them — which
/// the planner consumes positionally, also in graph order — keeps a surviving dedicated endpoint
/// on its previous port. This is **best-effort**: large topology changes that reorder the graph or
/// insert/remove dedicated endpoints can still shift the positional alignment. The shared listener
/// is stable without this; pinning here is purely for the secondary listeners.
pub fn pin_dedicated_ports(plan: &Plan) -> Vec<u16> {
    // Every listener that isn't the shared one (its host port is the gateway's host port) is a
    // dedicated listener, already in graph order on the plan.
    let shared = plan.gateway_host_port();
    plan.gateway
        .listeners
        .iter()
        .filter(|l| l.host_port != shared)
        .map(|l| l.host_port)
        .collect()
}

#[cfg(feature = "std-io")]
impl EnvManifest {
    /// Read and parse a manifest from `path`. Behind `std-io` (the only filesystem touch).
    pub fn read_from(path: &std::path::Path) -> Result<Self, ManifestError> {
        Self::from_toml(&std::fs::read_to_string(path)?)
    }

    /// Serialize this manifest and write it to `path`, creating parent directories as needed.
    /// Behind `std-io` (the only filesystem touch).
    pub fn write_to(&self, path: &std::path::Path) -> Result<(), ManifestError> {
        let toml = self.to_toml()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, toml)?;
        Ok(())
    }
}

/// A version-2 `[selection.external.<module>]` entry: an in-stack provider marked external.
#[derive(Deserialize)]
struct LegacyExternal {
    #[serde(default)]
    cloud: Option<String>,
    connection: LegacyConnection,
}

/// A version-2 external connection.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
enum LegacyConnection {
    RelationalDb {
        host: String,
        #[serde(default = "legacy_postgres_port")]
        port: u16,
        #[serde(default = "legacy_name_placeholder")]
        database: String,
        username: String,
        password: String,
    },
    ObjectStore {
        endpoint: String,
        #[serde(default)]
        uri: Option<String>,
        #[serde(default = "legacy_name_placeholder")]
        bucket: String,
        credential: ObjectStoreCredential,
    },
}

fn legacy_postgres_port() -> u16 {
    5432
}

fn legacy_name_placeholder() -> String {
    "{name}".into()
}

/// Rewrite a pre-version-3 manifest's `selection.external` table in place into
/// `selection.hosted`, swapping each in-stack module for its hosted provider. Returns whether
/// anything was migrated.
fn migrate_external(table: &mut toml::Table) -> Result<bool, ManifestError> {
    let Some(toml::Value::Table(selection)) = table.get_mut("selection") else {
        return Ok(false);
    };
    let Some(external) = selection.remove("external") else {
        return Ok(false);
    };
    let external: std::collections::BTreeMap<String, LegacyExternal> = external.try_into()?;
    for (module, legacy) in external {
        let (hosted_id, resource) = legacy_to_hosted(&module, legacy)?;
        let hosted = selection
            .entry("hosted")
            .or_insert_with(|| toml::Value::Table(toml::Table::new()));
        if let toml::Value::Table(hosted) = hosted {
            hosted.insert(hosted_id.into(), toml::Value::try_from(resource)?);
        }
        // The hosted provider replaces the in-stack one (and the hosted entry selects it).
        if let Some(toml::Value::Array(modules)) = selection.get_mut("modules") {
            modules.retain(|m| m.as_str() != Some(module.as_str()));
        }
        if let Some(toml::Value::Table(overrides)) = selection.get_mut("knob_overrides") {
            overrides.remove(&module);
        }
    }
    Ok(true)
}

/// The hosted provider and its configuration for a legacy external entry on `module`.
fn legacy_to_hosted(
    module: &str,
    legacy: LegacyExternal,
) -> Result<(&'static str, HostedResource), ManifestError> {
    let (id, connection) = match legacy.connection {
        LegacyConnection::RelationalDb {
            host,
            port,
            database,
            username,
            password,
        } => (
            "hosted-postgres",
            HostedConnection::Postgres {
                host,
                port,
                database,
                username,
                password,
            },
        ),
        LegacyConnection::ObjectStore {
            endpoint,
            bucket,
            credential:
                ObjectStoreCredential::S3 {
                    access_key_id,
                    secret_access_key,
                    region,
                    role_arn,
                },
            ..
        } => {
            // Keep only an endpoint the region doesn't already imply.
            let endpoint = endpoint.trim_end_matches('/');
            let endpoint = (endpoint != format!("https://s3.{region}.amazonaws.com"))
                .then(|| endpoint.to_string());
            (
                "aws-s3",
                HostedConnection::S3 {
                    region,
                    endpoint,
                    bucket,
                    access_key_id,
                    secret_access_key,
                    role_arn,
                },
            )
        }
        LegacyConnection::ObjectStore {
            endpoint,
            uri,
            bucket,
            credential: ObjectStoreCredential::AzureBlob { connection_string },
        } => {
            let account = azure_account(&endpoint)
                .or_else(|| uri.as_deref().and_then(azure_account))
                .ok_or_else(|| ManifestError::LegacyExternal {
                    module: module.into(),
                    reason: format!(
                        "no storage account in endpoint `{endpoint}`; configure \
                         `[selection.hosted.azure-storage]` by hand"
                    ),
                })?;
            (
                "azure-storage",
                HostedConnection::AzureBlob {
                    account,
                    container: bucket,
                    connection_string,
                },
            )
        }
    };
    Ok((
        id,
        HostedResource {
            cloud: legacy.cloud,
            connection,
        },
    ))
}

/// The storage account in an Azure URL: `https://<account>.blob.core.windows.net` or
/// `abfss://<container>@<account>.dfs.core.windows.net`.
fn azure_account(url: &str) -> Option<String> {
    let authority = url.split_once("://")?.1.split('/').next()?;
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let (account, domain) = host.split_once('.')?;
    (!account.is_empty() && domain.ends_with(".core.windows.net")).then(|| account.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::baseline::baseline_catalog;

    fn sample_manifest() -> EnvManifest {
        EnvManifest::new(
            Selection::modules(["envoy", "postgres", "rustfs", "unity-catalog", "mlflow"]),
            PlanCtx::default(),
        )
    }

    #[test]
    fn toml_round_trips_exactly() {
        let manifest = sample_manifest();
        let toml = manifest.to_toml().unwrap();
        let back = EnvManifest::from_toml(&toml).unwrap();
        assert_eq!(manifest, back);
    }

    #[test]
    fn partial_toml_fills_context_defaults() {
        // Only a selection; no [context] table at all.
        let toml = r#"
            [selection]
            modules = ["envoy", "postgres", "mlflow"]
        "#;
        let manifest = EnvManifest::from_toml(toml).unwrap();
        assert_eq!(manifest.version, EnvManifest::current_version());
        assert_eq!(manifest.context.gateway_host_port, 9080);
        assert_eq!(manifest.context.env_name, "lakehouse");
        // An older manifest with no `extra_resources` still parses (empty list).
        assert!(manifest.selection.extra_resources.is_empty());
    }

    #[test]
    fn extra_resource_helpers_are_idempotent_and_round_trip() {
        let mut manifest = sample_manifest();
        let extra = ExtraResource {
            resource: "object_store".into(),
            name: "exports".into(),
            provider: None,
        };
        manifest.add_extra_resource(extra.clone());
        manifest.add_extra_resource(extra.clone()); // idempotent on (resource, name)
        assert_eq!(manifest.selection.extra_resources, vec![extra.clone()]);

        // Survives a TOML round-trip.
        let back = EnvManifest::from_toml(&manifest.to_toml().unwrap()).unwrap();
        assert_eq!(back.selection.extra_resources, vec![extra]);

        manifest.remove_extra_resource("object_store", "exports");
        assert!(manifest.selection.extra_resources.is_empty());
    }

    #[test]
    fn saved_gateway_host_port_wins_over_default() {
        let mut manifest = sample_manifest();
        manifest.context.gateway_host_port = 9090;
        let toml = manifest.to_toml().unwrap();
        let reloaded = EnvManifest::from_toml(&toml).unwrap();
        let plan = reloaded.plan(&baseline_catalog()).unwrap();
        assert_eq!(plan.gateway_host_port(), 9090);
        // The shared listener (the one published on the host port) reflects it.
        assert!(
            plan.gateway.listeners.iter().any(|l| l.host_port == 9090),
            "shared listener should publish the saved host port"
        );
    }

    #[test]
    fn plan_is_deterministic_across_replans() {
        let manifest = sample_manifest();
        let catalog = baseline_catalog();
        let a = crate::render_all(&manifest.plan(&catalog).unwrap()).unwrap();
        let b = crate::render_all(&manifest.plan(&catalog).unwrap()).unwrap();
        assert_eq!(a.compose, b.compose);
        assert_eq!(a.envoy, b.envoy);
        assert_eq!(a.env, b.env);
    }

    #[test]
    fn editing_selection_keeps_shared_listener_stable() {
        let catalog = baseline_catalog();
        let mut manifest = sample_manifest();

        // Snapshot the shared listener (host port + its routes) before the edit.
        let before = manifest.plan(&catalog).unwrap();
        let shared_port = before.gateway_host_port();
        let shared_routes_before: Vec<_> = before
            .gateway
            .listeners
            .iter()
            .find(|l| l.host_port == shared_port)
            .unwrap()
            .routes
            .clone();

        // Add a module that contributes its own surface routes.
        manifest.add_module("headwaters");
        let after = manifest.plan(&catalog).unwrap();
        assert_eq!(
            after.gateway_host_port(),
            shared_port,
            "host port must not move"
        );
        let shared_after = after
            .gateway
            .listeners
            .iter()
            .find(|l| l.host_port == shared_port)
            .unwrap();
        // Every prior route survives with the same cluster + rewrite.
        for route in &shared_routes_before {
            assert!(
                shared_after.routes.contains(route),
                "shared route {route:?} should survive the add unchanged"
            );
        }

        // Removing the module restores the exact original shared listener.
        manifest.remove_module(&ModuleId::from("headwaters"));
        let restored = manifest.plan(&catalog).unwrap();
        let shared_restored = restored
            .gateway
            .listeners
            .iter()
            .find(|l| l.host_port == shared_port)
            .unwrap();
        assert_eq!(shared_restored.routes, shared_routes_before);
    }

    #[test]
    fn set_and_remove_knob_and_capability() {
        let mut m = EnvManifest::new(Selection::default(), PlanCtx::default());
        m.add_module("envoy");
        m.add_module("envoy"); // idempotent
        assert_eq!(m.selection.modules.len(), 1);
        m.set_knob("envoy", "auth", "true");
        assert_eq!(
            m.selection.knob_overrides[&ModuleId::from("envoy")].get("auth"),
            Some(&"true".to_string())
        );
        m.remove_module(&ModuleId::from("envoy"));
        assert!(m.selection.modules.is_empty());
        assert!(
            m.selection.knob_overrides.is_empty(),
            "knobs dropped with module"
        );

        m.add_capability("experiment_tracking");
        m.add_capability("experiment_tracking"); // idempotent
        assert_eq!(m.selection.capabilities.len(), 1);
        m.remove_capability("experiment_tracking");
        assert!(m.selection.capabilities.is_empty());
    }

    #[test]
    fn a_newer_manifest_is_rejected_rather_than_half_read() {
        let text = format!("version = {}\n", EnvManifest::current_version() + 1);
        assert!(matches!(
            EnvManifest::from_toml(&text),
            Err(ManifestError::UnsupportedVersion { .. })
        ));
        // Version-1 files (no `[selection.gateway]`) still read.
        let v1 =
            EnvManifest::from_toml("version = 1\n[selection]\nmodules = [\"envoy\"]\n").unwrap();
        assert_eq!(v1.selection.gateway, crate::GatewaySettings::OPEN);
    }

    #[test]
    fn normalizing_a_v1_manifest_moves_auth_into_gateway_settings() {
        let mut m = EnvManifest::from_toml(
            "version = 1\n[selection]\nmodules = [\"envoy\", \"mlflow\"]\n\
             [selection.knob_overrides.envoy]\nauth = \"true\"\nimage = \"envoy:pinned\"\n",
        )
        .unwrap();
        m.normalize_gateway(&baseline_catalog()).unwrap();
        assert_eq!(m.version, EnvManifest::current_version());
        assert_eq!(m.selection.gateway, crate::GatewaySettings::SIGN_IN);
        assert_eq!(m.selection.modules, vec![ModuleId::from("mlflow")]);
        // Only the legacy key moves; the gateway's other knobs stay.
        let envoy = &m.selection.knob_overrides[&ModuleId::from("envoy")];
        assert_eq!(envoy.len(), 1);
        assert_eq!(envoy["image"], "envoy:pinned");
        assert!(
            m.to_toml()
                .unwrap()
                .contains("[selection.gateway]\nauth = true\npolicy = false")
        );
    }

    #[test]
    fn a_v2_external_entry_migrates_to_its_hosted_provider() {
        let m = EnvManifest::from_toml(
            "version = 2\n[selection]\nmodules = [\"postgres\", \"rustfs\", \"unity-catalog\"]\n\
             [selection.knob_overrides.rustfs]\nimage = \"rustfs:pinned\"\n\
             [selection.external.postgres]\ncloud = \"aws\"\n\
             [selection.external.postgres.connection]\nkind = \"relational_db\"\n\
             host = \"db.example.com\"\nusername = \"app\"\npassword = \"${PG_PASSWORD}\"\n\
             [selection.external.rustfs.connection]\nkind = \"object_store\"\n\
             endpoint = \"https://s3.eu-west-1.amazonaws.com/\"\n\
             [selection.external.rustfs.connection.credential]\nflavour = \"s3\"\n\
             access_key_id = \"${AWS_ACCESS_KEY_ID}\"\n\
             secret_access_key = \"${AWS_SECRET_ACCESS_KEY}\"\nregion = \"eu-west-1\"\n",
        )
        .unwrap();
        assert_eq!(m.version, EnvManifest::current_version());
        assert_eq!(m.selection.modules, vec![ModuleId::from("unity-catalog")]);
        assert!(m.selection.knob_overrides.is_empty());
        let pg = &m.selection.hosted[&ModuleId::from("hosted-postgres")];
        assert_eq!(pg.cloud.as_deref(), Some("aws"));
        assert!(matches!(
            &pg.connection,
            HostedConnection::Postgres { host, port: 5432, .. } if host == "db.example.com"
        ));
        let s3 = &m.selection.hosted[&ModuleId::from("aws-s3")];
        assert!(matches!(
            &s3.connection,
            HostedConnection::S3 { region, endpoint: None, .. } if region == "eu-west-1"
        ));
        // It plans with the hosted providers in place of the in-stack ones.
        let plan = m.plan(&baseline_catalog()).unwrap();
        assert!(plan.graph.module(&ModuleId::from("rustfs")).is_none());
        assert!(plan.graph.module(&ModuleId::from("aws-s3")).is_some());
    }

    #[test]
    fn a_v2_azure_entry_migrates_with_its_account() {
        let m = EnvManifest::from_toml(
            "version = 2\n[selection]\nmodules = [\"azurite\"]\n\
             [selection.external.azurite.connection]\nkind = \"object_store\"\n\
             endpoint = \"https://lakeacct.blob.core.windows.net\"\n\
             uri = \"abfss://{name}@lakeacct.dfs.core.windows.net\"\n\
             [selection.external.azurite.connection.credential]\nflavour = \"azure_blob\"\n\
             connection_string = \"${AZURE_STORAGE_CONNECTION_STRING}\"\n",
        )
        .unwrap();
        assert!(m.selection.modules.is_empty());
        assert!(matches!(
            &m.selection.hosted[&ModuleId::from("azure-storage")].connection,
            HostedConnection::AzureBlob { account, .. } if account == "lakeacct"
        ));
        assert_eq!(azure_account("http://azurite:10000/devstoreaccount1"), None);
    }
}

#[cfg(all(test, feature = "std-io"))]
mod std_io_tests {
    use super::*;
    use crate::plan::Selection;

    #[test]
    fn write_then_read_round_trips() {
        let manifest = EnvManifest::new(
            Selection::modules(["envoy", "postgres", "mlflow"]),
            PlanCtx::default(),
        );
        let dir =
            std::env::temp_dir().join(format!("stack-topology-manifest-{}", std::process::id()));
        let path = dir.join("env.toml");
        let _ = std::fs::remove_dir_all(&dir);
        manifest.write_to(&path).unwrap();
        let back = EnvManifest::read_from(&path).unwrap();
        assert_eq!(manifest, back);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
