//! **External resources**: a resource provider the environment uses but does not run — a
//! managed cloud database or object store — in a stack whose other services run on the deploy
//! target.
//!
//! The user marks a provider module external ([`Selection::external`](super::Selection::external))
//! and says how to reach it ([`ExternalConnection`]). The provider stays in the graph — it still
//! satisfies its role's demands, and the topology still shows it — but:
//!
//! - its consumers bind the external connection instead of the provider's local one;
//! - its services are [`Placement::External`](crate::Placement::External): reached directly at
//!   the external origin from every vantage, never through the gateway (no routes, no dedicated
//!   listener, no emulated cloud hostnames);
//! - consumers don't wait on it to start, and the deploy target renders nothing for it. The
//!   resources it would provision must already exist.
//!
//! Secrets are **env-var references**, so `env.toml` stays committable: a secret-bearing field
//! holds `${VAR}`, and [`PlanCtx::secrets`](super::PlanCtx::secrets) — supplied at plan time by
//! the consumer (e.g. from the process environment) — resolves it. With no secrets supplied (a
//! browser preview) the references stay verbatim.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::catalog::module::ModuleId;
use crate::model::connection::{Connection, ConnectionTemplate, ObjectStoreCredential};

/// A provider module marked external: which cloud environment it lives in and how consumers
/// reach it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalResource {
    /// The cloud environment it lives in (e.g. `"aws"`, `"azure"`) — a free-form label that
    /// groups external resources in the topology. `None` for an unnamed external environment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud: Option<String>,
    /// How consumers reach it.
    pub connection: ExternalConnection,
}

/// How consumers reach an external resource. Mirrors the [`Connection`] variants, with the parts
/// a local provider derives (a database URL, a bucket URI) defaulted.
///
/// Every string may hold the `{name}` placeholder (substituted per demanded resource, like a
/// provider's [`ConnectionTemplate`]) and `${VAR}` secret references (resolved from
/// [`PlanCtx::secrets`](super::PlanCtx::secrets)).
///
/// `Debug` is hand-written to redact the database password (an object-store credential redacts
/// itself), since a literal secret may be written here.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
#[non_exhaustive]
pub enum ExternalConnection {
    /// A managed PostgreSQL server.
    RelationalDb {
        /// The server's host (e.g. `mydb.abc123.us-east-1.rds.amazonaws.com`).
        host: String,
        /// The server's port.
        #[serde(default = "default_postgres_port")]
        port: u16,
        /// The database to connect to; defaults to the demanded resource's name.
        #[serde(default = "name_placeholder")]
        database: String,
        /// The role to connect as.
        username: String,
        /// The role's password — typically a `${VAR}` reference.
        password: String,
    },
    /// A cloud object store.
    ObjectStore {
        /// The store's endpoint (e.g. `https://s3.us-east-1.amazonaws.com`).
        endpoint: String,
        /// The client-addressable URI for a resource. Defaults to `s3://{name}` for an S3
        /// credential; required for an Azure one (`abfss://{name}@<account>.dfs.core.windows.net`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        uri: Option<String>,
        /// The bucket/container name; defaults to the demanded resource's name.
        #[serde(default = "name_placeholder")]
        bucket: String,
        /// The credential consumers authenticate with — secrets typically `${VAR}` references.
        credential: ObjectStoreCredential,
    },
}

fn default_postgres_port() -> u16 {
    5432
}

fn name_placeholder() -> String {
    "{name}".into()
}

impl std::fmt::Debug for ExternalConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExternalConnection::RelationalDb {
                host,
                port,
                database,
                username,
                ..
            } => f
                .debug_struct("RelationalDb")
                .field("host", host)
                .field("port", port)
                .field("database", database)
                .field("username", username)
                .field("password", &"<redacted>")
                .finish(),
            ExternalConnection::ObjectStore {
                endpoint,
                uri,
                bucket,
                credential,
            } => f
                .debug_struct("ObjectStore")
                .field("endpoint", endpoint)
                .field("uri", uri)
                .field("bucket", bucket)
                .field("credential", credential)
                .finish(),
        }
    }
}

impl ExternalConnection {
    /// Whether this connection can stand in for a provider's `template` — the same
    /// [`Connection`] variant.
    pub(super) fn matches(&self, template: &ConnectionTemplate) -> bool {
        matches!(
            (self, &template.0),
            (
                ExternalConnection::RelationalDb { .. },
                Connection::RelationalDb { .. }
            ) | (
                ExternalConnection::ObjectStore { .. },
                Connection::ObjectStore { .. }
            )
        )
    }

    /// The service origin consumers reach: the object store's endpoint, or the database's
    /// `tcp://host:port`.
    pub(super) fn origin(&self) -> String {
        match self {
            ExternalConnection::RelationalDb { host, port, .. } => format!("tcp://{host}:{port}"),
            ExternalConnection::ObjectStore { endpoint, .. } => {
                endpoint.trim_end_matches('/').to_string()
            }
        }
    }

    /// The [`ConnectionTemplate`] consumers bind, or why this connection is incomplete.
    pub(super) fn template(&self) -> Result<ConnectionTemplate, String> {
        Ok(ConnectionTemplate(match self {
            ExternalConnection::RelationalDb {
                host,
                port,
                database,
                username,
                password,
            } => Connection::postgres(
                host.clone(),
                *port,
                database.clone(),
                username.clone(),
                password.clone(),
            ),
            ExternalConnection::ObjectStore {
                endpoint,
                uri,
                bucket,
                credential,
            } => {
                let uri = match (uri, credential) {
                    (Some(uri), _) => uri.clone(),
                    (None, ObjectStoreCredential::S3 { .. }) => "s3://{name}".into(),
                    (None, _) => {
                        return Err("an Azure object store needs an explicit `uri`".into());
                    }
                };
                Connection::ObjectStore {
                    uri,
                    bucket: bucket.clone(),
                    endpoint: endpoint.clone(),
                    credential: credential.clone(),
                    tls_trust: None,
                }
            }
        }))
    }

    /// Apply `f` to every string field (to scan or substitute secret references).
    fn map_strings(&self, mut f: impl FnMut(&str) -> String) -> ExternalConnection {
        match self {
            ExternalConnection::RelationalDb {
                host,
                port,
                database,
                username,
                password,
            } => ExternalConnection::RelationalDb {
                host: f(host),
                port: *port,
                database: f(database),
                username: f(username),
                password: f(password),
            },
            ExternalConnection::ObjectStore {
                endpoint,
                uri,
                bucket,
                credential,
            } => ExternalConnection::ObjectStore {
                endpoint: f(endpoint),
                uri: uri.as_deref().map(&mut f),
                bucket: f(bucket),
                credential: match credential {
                    ObjectStoreCredential::S3 {
                        access_key_id,
                        secret_access_key,
                        region,
                        role_arn,
                    } => ObjectStoreCredential::S3 {
                        access_key_id: f(access_key_id),
                        secret_access_key: f(secret_access_key),
                        region: f(region),
                        role_arn: role_arn.as_deref().map(&mut f),
                    },
                    ObjectStoreCredential::AzureBlob { connection_string } => {
                        ObjectStoreCredential::AzureBlob {
                            connection_string: f(connection_string),
                        }
                    }
                },
            },
        }
    }

    /// Every `${VAR}` secret reference in this connection, sorted.
    pub fn secret_refs(&self) -> BTreeSet<String> {
        let mut refs = BTreeSet::new();
        self.map_strings(|s| {
            refs.extend(env_refs(s).into_iter().map(|(_, name)| name.to_string()));
            String::new()
        });
        refs
    }

    /// Substitute every `${VAR}` from `secrets`, recording each resolved value in `values`
    /// (for redaction downstream) and each missing name in `missing`.
    fn resolve(
        &self,
        secrets: &BTreeMap<String, String>,
        values: &mut BTreeSet<String>,
        missing: &mut BTreeSet<String>,
    ) -> ExternalConnection {
        self.map_strings(|s| {
            let mut out = String::with_capacity(s.len());
            let mut rest = 0;
            for (span, name) in env_refs(s) {
                out.push_str(&s[rest..span.start]);
                match secrets.get(name) {
                    Some(value) => {
                        out.push_str(value);
                        if !value.is_empty() {
                            values.insert(value.clone());
                        }
                    }
                    None => {
                        missing.insert(name.to_string());
                        out.push_str(&s[span.clone()]);
                    }
                }
                rest = span.end;
            }
            out.push_str(&s[rest..]);
            out
        })
    }
}

/// The `${NAME}` references in `s`: each one's byte span and name. Only the bare `${NAME}` form
/// counts (an identifier, not starting with a digit).
fn env_refs(s: &str) -> Vec<(std::ops::Range<usize>, &str)> {
    let mut refs = Vec::new();
    let mut i = 0;
    while let Some(off) = s[i..].find("${") {
        let start = i + off;
        let name_start = start + 2;
        let Some(len) = s[name_start..].find('}') else {
            break;
        };
        let name = &s[name_start..name_start + len];
        let valid = !name.is_empty()
            && !name.as_bytes()[0].is_ascii_digit()
            && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
        if valid {
            refs.push((start..name_start + len + 1, name));
        }
        i = name_start;
    }
    refs
}

/// Secret values supplied at plan time, keyed by env-var name — what `${VAR}` references in an
/// [`ExternalConnection`] resolve against (see [`PlanCtx::secrets`](super::PlanCtx::secrets)).
///
/// `Debug` shows only the names, never the values.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Secrets(BTreeMap<String, String>);

impl Secrets {
    /// No secrets.
    pub fn new() -> Self {
        Secrets::default()
    }

    /// Set the secret `name` to `value`.
    pub fn insert(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.0.insert(name.into(), value.into());
    }

    /// The value of the secret `name`, if supplied.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }
}

impl<K: Into<String>, V: Into<String>> FromIterator<(K, V)> for Secrets {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        Secrets(
            iter.into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        )
    }
}

impl std::fmt::Debug for Secrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_set().entries(self.0.keys()).finish()
    }
}

/// The secret values a plan substituted into external connections, so a renderer can treat
/// any file containing one as sensitive. `Debug` shows only how many there are.
#[derive(Clone, Default)]
pub(crate) struct SecretValues(pub(crate) BTreeSet<String>);

impl SecretValues {
    /// Whether `text` contains any of the values.
    pub(crate) fn appear_in(&self, text: &str) -> bool {
        self.0.iter().any(|v| text.contains(v.as_str()))
    }
}

impl std::fmt::Debug for SecretValues {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SecretValues({} redacted)", self.0.len())
    }
}

/// An external provider, resolved for planning: its connection template with secrets
/// substituted, and where consumers reach it.
#[derive(Clone, Debug)]
pub(super) struct ResolvedExternal {
    /// The connection template consumers bind (secrets resolved).
    pub template: ConnectionTemplate,
    /// The origin its services are reached at.
    pub origin: String,
}

/// The external providers, resolved: the per-module [`ResolvedExternal`]s plus every secret
/// value substituted into them (so renderers can treat files that contain one as sensitive).
#[derive(Default)]
pub(super) struct Externals {
    pub modules: BTreeMap<ModuleId, ResolvedExternal>,
    pub secret_values: SecretValues,
}

impl Externals {
    /// Whether `module` is an external provider.
    pub fn contains(&self, module: &ModuleId) -> bool {
        self.modules.contains_key(module)
    }
}

/// Resolve every external resource of the selection against `secrets`.
///
/// `secrets: None` (a preview) leaves `${VAR}` references verbatim; `Some` resolves them and
/// collects every missing name into `Err(missing)`. A connection that is incomplete on its own
/// (e.g. an Azure store without a `uri`) is `Err` in `invalid`.
pub(super) fn resolve_externals(
    external: &BTreeMap<ModuleId, ExternalResource>,
    secrets: Option<&Secrets>,
) -> Result<Externals, ResolveExternalsError> {
    let mut out = Externals::default();
    let mut missing = BTreeSet::new();
    for (module, resource) in external {
        let connection = match secrets {
            Some(secrets) => {
                resource
                    .connection
                    .resolve(&secrets.0, &mut out.secret_values.0, &mut missing)
            }
            None => resource.connection.clone(),
        };
        let template = connection
            .template()
            .map_err(|reason| ResolveExternalsError::Invalid {
                module: module.clone(),
                reason,
            })?;
        out.modules.insert(
            module.clone(),
            ResolvedExternal {
                template,
                origin: connection.origin(),
            },
        );
    }
    if missing.is_empty() {
        Ok(out)
    } else {
        Err(ResolveExternalsError::Unresolved(
            missing.into_iter().collect(),
        ))
    }
}

/// Why [`resolve_externals`] failed.
pub(super) enum ResolveExternalsError {
    /// Secret references with no supplied value.
    Unresolved(Vec<String>),
    /// An external connection that is incomplete on its own.
    Invalid { module: ModuleId, reason: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rds() -> ExternalConnection {
        ExternalConnection::RelationalDb {
            host: "db.example.com".into(),
            port: 5432,
            database: "{name}".into(),
            username: "app".into(),
            password: "${PG_PASSWORD}".into(),
        }
    }

    #[test]
    fn secret_refs_are_found_and_resolved() {
        let c = rds();
        assert_eq!(
            c.secret_refs().into_iter().collect::<Vec<_>>(),
            vec!["PG_PASSWORD".to_string()]
        );
        let value = "resolved-value";
        let secrets = BTreeMap::from([("PG_PASSWORD".to_string(), value.to_string())]);
        let (mut values, mut missing) = (BTreeSet::new(), BTreeSet::new());
        let resolved = c.resolve(&secrets, &mut values, &mut missing);
        assert!(missing.is_empty());
        assert!(values.contains(value));
        let ConnectionTemplate(Connection::RelationalDb { url, password, .. }) =
            resolved.template().unwrap()
        else {
            panic!("relational template");
        };
        assert_eq!(password, value);
        // The derived URL embeds the resolved value, not the reference.
        assert!(url.contains(value) && !url.contains("${"), "{url}");
        assert!(url.ends_with("@db.example.com:5432/{name}"), "{url}");
    }

    #[test]
    fn missing_secrets_are_reported_and_left_verbatim() {
        let (mut values, mut missing) = (BTreeSet::new(), BTreeSet::new());
        let resolved = rds().resolve(&BTreeMap::new(), &mut values, &mut missing);
        assert_eq!(missing.into_iter().collect::<Vec<_>>(), vec!["PG_PASSWORD"]);
        assert_eq!(resolved, rds());
    }

    #[test]
    fn env_refs_ignore_malformed_references() {
        let refs: Vec<&str> = env_refs("a${OK}b${1BAD}c${}d${ALSO_OK}${unclosed")
            .into_iter()
            .map(|(_, n)| n)
            .collect();
        assert_eq!(refs, vec!["OK", "ALSO_OK"]);
    }

    #[test]
    fn debug_redacts_the_password() {
        let debug = format!("{:?}", rds());
        assert!(!debug.contains("PG_PASSWORD"));
        assert!(debug.contains("<redacted>"));
    }

    #[test]
    fn an_azure_store_needs_an_explicit_uri() {
        let azure = ExternalConnection::ObjectStore {
            endpoint: "https://acct.blob.core.windows.net".into(),
            uri: None,
            bucket: "{name}".into(),
            credential: ObjectStoreCredential::AzureBlob {
                connection_string: "${AZURE_STORAGE_CONNECTION_STRING}".into(),
            },
        };
        assert!(azure.template().is_err());
    }

    #[test]
    fn origin_is_the_endpoint_or_a_tcp_address() {
        assert_eq!(rds().origin(), "tcp://db.example.com:5432");
        let s3 = ExternalConnection::ObjectStore {
            endpoint: "https://s3.us-east-1.amazonaws.com/".into(),
            uri: None,
            bucket: "{name}".into(),
            credential: ObjectStoreCredential::S3 {
                access_key_id: "${AWS_ACCESS_KEY_ID}".into(),
                secret_access_key: "${AWS_SECRET_ACCESS_KEY}".into(),
                region: "us-east-1".into(),
                role_arn: None,
            },
        };
        assert_eq!(s3.origin(), "https://s3.us-east-1.amazonaws.com");
    }
}
