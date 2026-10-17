//! **Hosted resources**: a resource provider the environment uses but does not run — a managed
//! Postgres server, an S3 bucket, an Azure storage account — in a stack whose other services run
//! on the deploy target.
//!
//! A hosted provider is its own catalog module (`hosted-postgres`, `aws-s3`, `azure-storage`),
//! an alternative to the in-stack provider of the same role (`postgres`, `rustfs`, `azurite`):
//! an environment has exactly one provider per role, so it is either the in-stack one or the
//! hosted one. The module declares its [`HostedKind`]; the planner then
//!
//! - binds its consumers to a [`HostedConnection`] — the selection's
//!   [`hosted`](super::Selection::hosted) entry for it, or the kind's
//!   [default](HostedKind::default_connection) built from the conventional env vars;
//! - places its services [`Placement::External`](crate::Placement::External): reached directly
//!   at the connection's origin from every vantage, never through the gateway;
//! - renders nothing for it. The resources it would provision are created by its
//!   [deploy steps](crate::catalog::deploy), which depend on the resource's [`HostedVendor`].
//!
//! Secrets are **env-var references**, so `env.toml` stays committable: a secret-bearing field
//! holds `${VAR}`, and [`PlanCtx::secrets`](super::PlanCtx::secrets) — supplied at plan time by
//! the consumer (e.g. from the process environment) — resolves it. With no secrets supplied (a
//! browser preview) the references stay verbatim.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::catalog::module::ModuleId;
use crate::model::connection::{Connection, ConnectionTemplate, ObjectStoreCredential};
use crate::model::role::Role;

/// What a hosted provider module stands in for — and so which [`HostedConnection`] variant
/// configures it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum HostedKind {
    /// A managed PostgreSQL server (Amazon RDS, Azure Database for PostgreSQL, Neon, …).
    Postgres,
    /// Amazon S3, or an S3-compatible store at an explicit endpoint.
    S3,
    /// An Azure storage account (Blob / ADLS Gen2).
    AzureBlob,
}

impl HostedKind {
    /// The kind's serialized name (`postgres`, `s3`, `azure_blob`).
    pub fn as_str(self) -> &'static str {
        match self {
            HostedKind::Postgres => "postgres",
            HostedKind::S3 => "s3",
            HostedKind::AzureBlob => "azure_blob",
        }
    }

    /// The resource role the kind provides.
    pub fn role(self) -> &'static str {
        match self {
            HostedKind::Postgres => Role::RELATIONAL_DB,
            HostedKind::S3 | HostedKind::AzureBlob => Role::OBJECT_STORE,
        }
    }

    /// The cloud a resource of this kind lives in, when the kind implies one — the label
    /// external nodes are grouped by in the topology. A hosted Postgres can be anywhere.
    pub fn default_cloud(self) -> Option<&'static str> {
        match self {
            HostedKind::Postgres => None,
            HostedKind::S3 => Some("aws"),
            HostedKind::AzureBlob => Some("azure"),
        }
    }

    /// The connection used when the selection configures none: every coordinate a `${VAR}`
    /// reference to the variable its ecosystem conventionally reads (`PGHOST`, `PGUSER`,
    /// `PGPASSWORD`; `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`; `AZURE_STORAGE_ACCOUNT`,
    /// `AZURE_STORAGE_CONNECTION_STRING`).
    pub fn default_connection(self) -> HostedConnection {
        match self {
            HostedKind::Postgres => HostedConnection::Postgres {
                host: "${PGHOST}".into(),
                port: default_postgres_port(),
                database: name_placeholder(),
                username: "${PGUSER}".into(),
                password: "${PGPASSWORD}".into(),
                ssl_mode: None,
            },
            HostedKind::S3 => HostedConnection::S3 {
                region: default_aws_region(),
                endpoint: None,
                bucket: name_placeholder(),
                access_key_id: "${AWS_ACCESS_KEY_ID}".into(),
                secret_access_key: "${AWS_SECRET_ACCESS_KEY}".into(),
                role_arn: None,
            },
            HostedKind::AzureBlob => HostedConnection::AzureBlob {
                account: "${AZURE_STORAGE_ACCOUNT}".into(),
                container: name_placeholder(),
                connection_string: "${AZURE_STORAGE_CONNECTION_STRING}".into(),
            },
        }
    }

    /// The default [`HostedResource`]: the [default connection](Self::default_connection) in
    /// the [default cloud](Self::default_cloud).
    pub fn default_resource(self) -> HostedResource {
        HostedResource {
            cloud: self.default_cloud().map(Into::into),
            vendor: Some(self.default_vendor()),
            connection: self.default_connection(),
        }
    }

    /// The vendor assumed when the selection names none: AWS for S3, Azure for Azure storage,
    /// and an unspecified server for Postgres.
    pub fn default_vendor(self) -> HostedVendor {
        match self {
            HostedKind::Postgres => HostedVendor::Generic,
            HostedKind::S3 => HostedVendor::AwsS3,
            HostedKind::AzureBlob => HostedVendor::AzureStorage,
        }
    }
}

/// Who runs a hosted resource: what its [deploy steps](crate::DeployStepSpec) provision it
/// with, and the connection defaults that vendor needs (managed Postgres services require TLS).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum HostedVendor {
    /// Any provider of the kind: setup is described, not scripted (beyond what works against
    /// any server, such as creating databases). For S3, an S3-compatible store.
    Generic,
    /// A Neon serverless Postgres project.
    Neon,
    /// An Amazon RDS for PostgreSQL instance.
    AwsRds,
    /// An Azure Database for PostgreSQL flexible server.
    AzurePostgres,
    /// Amazon S3.
    AwsS3,
    /// An Azure storage account.
    AzureStorage,
}

impl HostedVendor {
    /// Every vendor, in display order.
    pub const ALL: [HostedVendor; 6] = [
        HostedVendor::Generic,
        HostedVendor::Neon,
        HostedVendor::AwsRds,
        HostedVendor::AzurePostgres,
        HostedVendor::AwsS3,
        HostedVendor::AzureStorage,
    ];

    /// The vendor's serialized name (`generic`, `neon`, `aws-rds`, …).
    pub fn as_str(self) -> &'static str {
        match self {
            HostedVendor::Generic => "generic",
            HostedVendor::Neon => "neon",
            HostedVendor::AwsRds => "aws-rds",
            HostedVendor::AzurePostgres => "azure-postgres",
            HostedVendor::AwsS3 => "aws-s3",
            HostedVendor::AzureStorage => "azure-storage",
        }
    }

    /// A human-readable name for a wizard/UI.
    pub fn display_name(self) -> &'static str {
        match self {
            HostedVendor::Generic => "Other",
            HostedVendor::Neon => "Neon",
            HostedVendor::AwsRds => "Amazon RDS",
            HostedVendor::AzurePostgres => "Azure Database for PostgreSQL",
            HostedVendor::AwsS3 => "Amazon S3",
            HostedVendor::AzureStorage => "Azure Storage",
        }
    }

    /// Whether this vendor offers resources of `kind`. [`Generic`](Self::Generic) fits every
    /// kind.
    pub fn offers(self, kind: HostedKind) -> bool {
        match self {
            HostedVendor::Generic => true,
            HostedVendor::Neon | HostedVendor::AwsRds | HostedVendor::AzurePostgres => {
                kind == HostedKind::Postgres
            }
            HostedVendor::AwsS3 => kind == HostedKind::S3,
            HostedVendor::AzureStorage => kind == HostedKind::AzureBlob,
        }
    }

    /// The cloud label a resource of this vendor is grouped under, when the selection sets
    /// none.
    pub fn default_cloud(self) -> Option<&'static str> {
        match self {
            HostedVendor::Generic => None,
            HostedVendor::Neon => Some("neon"),
            HostedVendor::AwsRds | HostedVendor::AwsS3 => Some("aws"),
            HostedVendor::AzurePostgres | HostedVendor::AzureStorage => Some("azure"),
        }
    }

    /// The Postgres `sslmode` this vendor's servers need, when the connection sets none: the
    /// managed services all refuse unencrypted connections.
    pub fn default_ssl_mode(self) -> Option<&'static str> {
        match self {
            HostedVendor::Neon | HostedVendor::AwsRds | HostedVendor::AzurePostgres => {
                Some("require")
            }
            _ => None,
        }
    }
}

/// How a hosted provider module is configured: which cloud environment it lives in and how
/// consumers reach it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostedResource {
    /// The cloud environment it lives in (e.g. `"aws"`, `"azure"`, `"neon"`) — a free-form
    /// label that groups hosted resources in the topology. `None` for an unnamed environment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud: Option<String>,
    /// Who runs it, which decides its setup steps; defaults to the kind's
    /// [default vendor](HostedKind::default_vendor). The planner fills it in, along with a
    /// `cloud` derived from it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor: Option<HostedVendor>,
    /// How consumers reach it.
    pub connection: HostedConnection,
}

impl HostedResource {
    /// The effective vendor: the configured one, else the kind's default.
    pub fn vendor(&self) -> HostedVendor {
        self.vendor
            .unwrap_or_else(|| self.connection.kind().default_vendor())
    }

    /// This resource with every default the planner derives filled in: the vendor, the cloud
    /// label from it, and a Postgres `sslmode` the vendor requires.
    fn settled(mut self) -> HostedResource {
        let vendor = self.vendor();
        self.vendor = Some(vendor);
        if self.cloud.is_none() {
            self.cloud = vendor.default_cloud().map(Into::into);
        }
        if let HostedConnection::Postgres { ssl_mode, .. } = &mut self.connection
            && ssl_mode.is_none()
        {
            *ssl_mode = vendor.default_ssl_mode().map(Into::into);
        }
        self
    }

    /// The resource as a deploy step sees it (see [`HostedStepView`]), for the consumers'
    /// demanded resource names `objects`.
    pub fn step_view(&self, objects: &[String]) -> HostedStepView {
        let (fields, name_field): (Vec<StepField>, &str) = match &self.connection {
            HostedConnection::Postgres {
                host,
                port,
                database,
                username,
                password,
                ssl_mode,
            } => {
                let mut fields = vec![
                    ("host", host.clone(), None),
                    ("port", port.to_string(), None),
                    ("database", database.clone(), None),
                    ("username", username.clone(), None),
                    ("password", password.clone(), Some("PGPASSWORD")),
                ];
                fields.extend(ssl_mode.iter().map(|m| ("ssl_mode", m.clone(), None)));
                (fields, database)
            }
            HostedConnection::S3 {
                region,
                endpoint,
                bucket,
                access_key_id,
                secret_access_key,
                role_arn,
            } => {
                let mut fields = vec![
                    ("region", region.clone(), None),
                    ("bucket", bucket.clone(), None),
                    (
                        "access_key_id",
                        access_key_id.clone(),
                        Some("AWS_ACCESS_KEY_ID"),
                    ),
                    (
                        "secret_access_key",
                        secret_access_key.clone(),
                        Some("AWS_SECRET_ACCESS_KEY"),
                    ),
                ];
                fields.extend(endpoint.iter().map(|e| ("endpoint", e.clone(), None)));
                fields.extend(role_arn.iter().map(|a| ("role_arn", a.clone(), None)));
                (fields, bucket)
            }
            HostedConnection::AzureBlob {
                account,
                container,
                connection_string,
            } => (
                vec![
                    ("account", account.clone(), None),
                    ("container", container.clone(), None),
                    (
                        "connection_string",
                        connection_string.clone(),
                        Some("AZURE_STORAGE_CONNECTION_STRING"),
                    ),
                ],
                container,
            ),
        };
        let mut values = BTreeMap::new();
        let mut vars = BTreeMap::new();
        for (field, value, conventional) in fields {
            // A secret field set inline maps to its conventional variable, so a script reads
            // the environment rather than embedding the value.
            if let Some(var) = single_ref(&value).or(conventional) {
                vars.insert(field, var.to_string());
            }
            if conventional.is_none() {
                values.insert(field, value);
            }
        }
        HostedStepView {
            kind: self.connection.kind(),
            vendor: self.vendor(),
            cloud: self.cloud.clone(),
            values,
            vars,
            resources: objects
                .iter()
                .map(|o| name_field.replace("{name}", o))
                .collect(),
            secrets: self.connection.secret_refs().into_iter().collect(),
        }
    }
}

/// A hosted resource as its [deploy steps](crate::DeployStepSpec) see it: the `hosted`
/// template global. It carries coordinates and the *names* of the variables secrets are read
/// from, never a secret's value, so rendered instructions and scripts are safe to commit and
/// show.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HostedStepView {
    /// The resource kind (`postgres`, `s3`, `azure_blob`).
    pub kind: HostedKind,
    /// The effective vendor (`neon`, `aws-rds`, `aws-s3`, …).
    pub vendor: HostedVendor,
    /// The cloud label, if any.
    pub cloud: Option<String>,
    /// The connection's non-secret fields as configured (`host`, `port`, `database`,
    /// `username`, `ssl_mode`; `region`, `endpoint`, `bucket`, `role_arn`; `account`,
    /// `container`). A `${VAR}` reference stays verbatim, which a shell expands.
    pub values: BTreeMap<&'static str, String>,
    /// The variable each field is read from: every field that is exactly one `${VAR}`, and
    /// every secret field (`password`, `access_key_id`, `secret_access_key`,
    /// `connection_string`), whose variable is the conventional one when it's set inline.
    pub vars: BTreeMap<&'static str, String>,
    /// The resources consumers demand, by their real names: each demanded name substituted
    /// into the connection's database / bucket / container field.
    pub resources: Vec<String>,
    /// Every `${VAR}` the connection references, sorted.
    pub secrets: Vec<String>,
}

/// One connection field as [`HostedResource::step_view`] reads it: its name, its configured
/// value, and, for a secret field, the conventional variable it is read from when set inline.
type StepField = (&'static str, String, Option<&'static str>);

/// The variable `s` reads when it is exactly one `${VAR}` reference.
fn single_ref(s: &str) -> Option<&str> {
    match env_refs(s).as_slice() {
        [(span, name)] if *span == (0..s.len()) => Some(name),
        _ => None,
    }
}

/// How consumers reach a hosted resource, one variant per [`HostedKind`]. The parts a cloud
/// derives by convention (an S3 endpoint from its region, Azure URLs from the account name)
/// are derived here too.
///
/// Every string may hold the `{name}` placeholder (substituted per demanded resource, like a
/// provider's [`ConnectionTemplate`]) and `${VAR}` secret references (resolved from
/// [`PlanCtx::secrets`](super::PlanCtx::secrets)).
///
/// `Debug` is hand-written to redact every secret-bearing field, since a literal secret may be
/// written here.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
#[non_exhaustive]
pub enum HostedConnection {
    /// A managed PostgreSQL server.
    Postgres {
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
        /// The libpq `sslmode` consumers connect with (e.g. `require`); defaults to what the
        /// [vendor](HostedVendor::default_ssl_mode) needs, else the client's default.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ssl_mode: Option<String>,
    },
    /// Amazon S3 (or an S3-compatible store, via `endpoint`).
    S3 {
        /// The bucket region.
        #[serde(default = "default_aws_region")]
        region: String,
        /// The S3 API endpoint; defaults to `https://s3.<region>.amazonaws.com`. Set it for an
        /// S3-compatible store (Cloudflare R2, a remote MinIO, …).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        endpoint: Option<String>,
        /// The bucket name; defaults to the demanded resource's name.
        #[serde(default = "name_placeholder")]
        bucket: String,
        /// The access key id — typically a `${VAR}` reference.
        access_key_id: String,
        /// The secret access key — typically a `${VAR}` reference.
        secret_access_key: String,
        /// An IAM role consumers may assume to vend scoped, temporary credentials.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        role_arn: Option<String>,
    },
    /// An Azure storage account (Blob / ADLS Gen2).
    AzureBlob {
        /// The storage account name; the blob endpoint and `abfss://` URIs derive from it.
        account: String,
        /// The container name; defaults to the demanded resource's name.
        #[serde(default = "name_placeholder")]
        container: String,
        /// The account's connection string — typically a `${VAR}` reference.
        connection_string: String,
    },
}

fn default_postgres_port() -> u16 {
    5432
}

fn default_aws_region() -> String {
    "us-east-1".into()
}

fn name_placeholder() -> String {
    "{name}".into()
}

impl std::fmt::Debug for HostedConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HostedConnection::Postgres {
                host,
                port,
                database,
                username,
                ssl_mode,
                ..
            } => f
                .debug_struct("Postgres")
                .field("host", host)
                .field("port", port)
                .field("database", database)
                .field("username", username)
                .field("password", &"<redacted>")
                .field("ssl_mode", ssl_mode)
                .finish(),
            HostedConnection::S3 {
                region,
                endpoint,
                bucket,
                role_arn,
                ..
            } => f
                .debug_struct("S3")
                .field("region", region)
                .field("endpoint", endpoint)
                .field("bucket", bucket)
                .field("access_key_id", &"<redacted>")
                .field("secret_access_key", &"<redacted>")
                .field("role_arn", role_arn)
                .finish(),
            HostedConnection::AzureBlob {
                account, container, ..
            } => f
                .debug_struct("AzureBlob")
                .field("account", account)
                .field("container", container)
                .field("connection_string", &"<redacted>")
                .finish(),
        }
    }
}

impl HostedConnection {
    /// The [`HostedKind`] this connection configures.
    pub fn kind(&self) -> HostedKind {
        match self {
            HostedConnection::Postgres { .. } => HostedKind::Postgres,
            HostedConnection::S3 { .. } => HostedKind::S3,
            HostedConnection::AzureBlob { .. } => HostedKind::AzureBlob,
        }
    }

    /// The service origin consumers reach: the object store's endpoint, or the database's
    /// `tcp://host:port`.
    pub(super) fn origin(&self) -> String {
        match self {
            HostedConnection::Postgres { host, port, .. } => format!("tcp://{host}:{port}"),
            HostedConnection::S3 {
                region, endpoint, ..
            } => s3_endpoint(region, endpoint.as_deref()),
            HostedConnection::AzureBlob { account, .. } => azure_blob_endpoint(account),
        }
    }

    /// The [`ConnectionTemplate`] consumers bind.
    pub(crate) fn template(&self) -> ConnectionTemplate {
        ConnectionTemplate(match self {
            HostedConnection::Postgres {
                host,
                port,
                database,
                username,
                password,
                ssl_mode,
            } => Connection::postgres(
                host.clone(),
                *port,
                database.clone(),
                username.clone(),
                password.clone(),
            )
            .with_ssl_mode(ssl_mode.clone()),
            HostedConnection::S3 {
                region,
                endpoint,
                bucket,
                access_key_id,
                secret_access_key,
                role_arn,
            } => Connection::ObjectStore {
                uri: format!("s3://{bucket}"),
                bucket: bucket.clone(),
                endpoint: s3_endpoint(region, endpoint.as_deref()),
                credential: ObjectStoreCredential::S3 {
                    access_key_id: access_key_id.clone(),
                    secret_access_key: secret_access_key.clone(),
                    region: region.clone(),
                    role_arn: role_arn.clone(),
                },
                tls_trust: None,
            },
            HostedConnection::AzureBlob {
                account,
                container,
                connection_string,
            } => Connection::ObjectStore {
                uri: format!("abfss://{container}@{account}.dfs.core.windows.net"),
                bucket: container.clone(),
                endpoint: azure_blob_endpoint(account),
                credential: ObjectStoreCredential::AzureBlob {
                    connection_string: connection_string.clone(),
                },
                tls_trust: None,
            },
        })
    }

    /// The coordinates a *client* of this resource (an app, a notebook) loads to reach it, as
    /// `(env var, value)` — never a credential, and never a value holding a `${VAR}` reference
    /// (the client environment is written unresolved and shared).
    pub(super) fn client_env(&self) -> Vec<(&'static str, String)> {
        let coordinates = match self {
            HostedConnection::Postgres { .. } => vec![],
            HostedConnection::S3 { region, .. } => vec![
                ("AWS_ENDPOINT_URL_S3", self.origin()),
                ("AWS_REGION", region.clone()),
            ],
            HostedConnection::AzureBlob { account, .. } => {
                vec![("AZURE_STORAGE_ACCOUNT", account.clone())]
            }
        };
        coordinates
            .into_iter()
            .filter(|(_, value)| env_refs(value).is_empty())
            .collect()
    }

    /// Apply `f` to every string field (to scan or substitute secret references).
    fn map_strings(&self, mut f: impl FnMut(&str) -> String) -> HostedConnection {
        match self {
            HostedConnection::Postgres {
                host,
                port,
                database,
                username,
                password,
                ssl_mode,
            } => HostedConnection::Postgres {
                host: f(host),
                port: *port,
                database: f(database),
                username: f(username),
                password: f(password),
                ssl_mode: ssl_mode.as_deref().map(&mut f),
            },
            HostedConnection::S3 {
                region,
                endpoint,
                bucket,
                access_key_id,
                secret_access_key,
                role_arn,
            } => HostedConnection::S3 {
                region: f(region),
                endpoint: endpoint.as_deref().map(&mut f),
                bucket: f(bucket),
                access_key_id: f(access_key_id),
                secret_access_key: f(secret_access_key),
                role_arn: role_arn.as_deref().map(&mut f),
            },
            HostedConnection::AzureBlob {
                account,
                container,
                connection_string,
            } => HostedConnection::AzureBlob {
                account: f(account),
                container: f(container),
                connection_string: f(connection_string),
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
    ) -> HostedConnection {
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

/// An S3 endpoint: the explicit one (without a trailing `/`), else AWS's regional endpoint.
fn s3_endpoint(region: &str, endpoint: Option<&str>) -> String {
    match endpoint {
        Some(endpoint) => endpoint.trim_end_matches('/').to_string(),
        None => format!("https://s3.{region}.amazonaws.com"),
    }
}

/// An Azure storage account's blob endpoint.
fn azure_blob_endpoint(account: &str) -> String {
    format!("https://{account}.blob.core.windows.net")
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
/// [`HostedConnection`] resolve against (see [`PlanCtx::secrets`](super::PlanCtx::secrets)).
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

/// The secret values a plan substituted into hosted connections, so a renderer can treat
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

/// A hosted provider, resolved for planning: its connection template with secrets substituted,
/// and where consumers reach it.
#[derive(Clone, Debug)]
pub(super) struct ResolvedHosted {
    /// The connection template consumers bind (secrets resolved).
    pub template: ConnectionTemplate,
    /// The origin its services are reached at.
    pub origin: String,
}

/// The hosted providers in the graph, resolved: the per-module [`ResolvedHosted`]s plus every
/// secret value substituted into them (so renderers can treat files that contain one as
/// sensitive).
#[derive(Default)]
pub(super) struct HostedProviders {
    /// Each hosted provider's effective configuration (the selection's, or the kind's default),
    /// secrets unresolved — what [`Plan::hosted`](super::Plan::hosted) exposes.
    pub resources: BTreeMap<ModuleId, HostedResource>,
    pub modules: BTreeMap<ModuleId, ResolvedHosted>,
    pub secret_values: SecretValues,
}

/// Resolve every hosted provider (module id → its kind) against the selection's `configured`
/// entries and `secrets`.
///
/// A provider with no entry uses its kind's [default](HostedKind::default_resource). An entry
/// whose connection is for another kind is `Err(Invalid)`. `secrets: None` (a preview) leaves
/// `${VAR}` references verbatim; `Some` resolves them and collects every missing name into
/// `Err(Unresolved)`.
pub(super) fn resolve_hosted(
    providers: &BTreeMap<ModuleId, HostedKind>,
    configured: &BTreeMap<ModuleId, HostedResource>,
    secrets: Option<&Secrets>,
) -> Result<HostedProviders, ResolveHostedError> {
    let mut out = HostedProviders::default();
    let mut missing = BTreeSet::new();
    for (module, kind) in providers {
        let resource = match configured.get(module) {
            Some(resource) if resource.connection.kind() != *kind => {
                return Err(ResolveHostedError::Invalid {
                    module: module.clone(),
                    reason: format!(
                        "it is a `{}` provider, but the connection is for `{}`",
                        kind.as_str(),
                        resource.connection.kind().as_str()
                    ),
                });
            }
            Some(resource) if !resource.vendor().offers(*kind) => {
                return Err(ResolveHostedError::Invalid {
                    module: module.clone(),
                    reason: format!(
                        "vendor `{}` offers no `{}` resources",
                        resource.vendor().as_str(),
                        kind.as_str()
                    ),
                });
            }
            Some(resource) => resource.clone().settled(),
            None => kind.default_resource().settled(),
        };
        let connection = match secrets {
            Some(secrets) => {
                resource
                    .connection
                    .resolve(&secrets.0, &mut out.secret_values.0, &mut missing)
            }
            None => resource.connection.clone(),
        };
        out.modules.insert(
            module.clone(),
            ResolvedHosted {
                template: connection.template(),
                origin: connection.origin(),
            },
        );
        out.resources.insert(module.clone(), resource);
    }
    if missing.is_empty() {
        Ok(out)
    } else {
        Err(ResolveHostedError::Unresolved(
            missing.into_iter().collect(),
        ))
    }
}

/// Why [`resolve_hosted`] failed.
pub(super) enum ResolveHostedError {
    /// Secret references with no supplied value.
    Unresolved(Vec<String>),
    /// A configured connection that doesn't fit its module.
    Invalid { module: ModuleId, reason: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rds() -> HostedConnection {
        HostedConnection::Postgres {
            host: "db.example.com".into(),
            port: 5432,
            database: "{name}".into(),
            username: "app".into(),
            password: "${PG_PASSWORD}".into(),
            ssl_mode: None,
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
            resolved.template()
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
    fn debug_redacts_secrets() {
        let debug = format!("{:?}", rds());
        assert!(!debug.contains("PG_PASSWORD"));
        assert!(debug.contains("<redacted>"));
        let debug = format!("{:?}", HostedKind::S3.default_connection());
        assert!(!debug.contains("AWS_SECRET_ACCESS_KEY"), "{debug}");
        let debug = format!("{:?}", HostedKind::AzureBlob.default_connection());
        assert!(!debug.contains("CONNECTION_STRING"), "{debug}");
    }

    #[test]
    fn azure_urls_derive_from_the_account() {
        let azure = HostedConnection::AzureBlob {
            account: "acct".into(),
            container: "{name}".into(),
            connection_string: "${AZURE_STORAGE_CONNECTION_STRING}".into(),
        };
        assert_eq!(azure.origin(), "https://acct.blob.core.windows.net");
        let ConnectionTemplate(Connection::ObjectStore { uri, .. }) = azure.template() else {
            panic!("object-store template");
        };
        assert_eq!(uri, "abfss://{name}@acct.dfs.core.windows.net");
    }

    #[test]
    fn s3_endpoint_derives_from_the_region_unless_set() {
        assert_eq!(rds().origin(), "tcp://db.example.com:5432");
        let HostedConnection::S3 {
            access_key_id,
            secret_access_key,
            ..
        } = HostedKind::S3.default_connection()
        else {
            panic!("s3 default");
        };
        let s3 = |region: &str, endpoint: Option<&str>| HostedConnection::S3 {
            region: region.into(),
            endpoint: endpoint.map(Into::into),
            bucket: "{name}".into(),
            access_key_id: access_key_id.clone(),
            secret_access_key: secret_access_key.clone(),
            role_arn: None,
        };
        assert_eq!(
            s3("eu-west-1", None).origin(),
            "https://s3.eu-west-1.amazonaws.com"
        );
        assert_eq!(
            s3("auto", Some("https://acct.r2.cloudflarestorage.com/")).origin(),
            "https://acct.r2.cloudflarestorage.com"
        );
    }

    #[test]
    fn client_env_has_coordinates_but_no_references() {
        assert_eq!(
            HostedKind::S3.default_connection().client_env(),
            vec![
                (
                    "AWS_ENDPOINT_URL_S3",
                    "https://s3.us-east-1.amazonaws.com".into()
                ),
                ("AWS_REGION", "us-east-1".to_string()),
            ]
        );
        // The default account is a `${VAR}` reference: not written for clients.
        assert!(
            HostedKind::AzureBlob
                .default_connection()
                .client_env()
                .is_empty()
        );
        assert!(rds().client_env().is_empty());
    }

    #[test]
    fn a_connection_for_another_kind_is_invalid() {
        let providers = BTreeMap::from([(ModuleId::from("aws-s3"), HostedKind::S3)]);
        let configured = BTreeMap::from([(
            ModuleId::from("aws-s3"),
            HostedResource {
                cloud: None,
                vendor: None,
                connection: rds(),
            },
        )]);
        assert!(matches!(
            resolve_hosted(&providers, &configured, None),
            Err(ResolveHostedError::Invalid { .. })
        ));
        // Unconfigured, the kind's default applies.
        let out = resolve_hosted(&providers, &BTreeMap::new(), None)
            .ok()
            .unwrap();
        assert_eq!(
            out.resources[&ModuleId::from("aws-s3")],
            HostedKind::S3.default_resource()
        );
    }
}
