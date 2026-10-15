//! The typed connection a resource demand negotiates ([`Connection`]).
//!
//! When a module demands a resource (a relational database, an object-store bucket, …),
//! the only thing that has to be negotiated is *how to connect*: a URL/endpoint plus a
//! credential. The credential shape is closed per flavour — an S3 store needs an access
//! key id and secret (plus region); an Azure Blob store needs a connection string; a
//! relational store carries its coordinates and credential both as one URL and as typed
//! parts. There are only a handful, so they are a
//! **typed enum** the compiler enforces, not an open string→value map.
//!
//! This is the deliberate trade the crate makes: a new resource *flavour* (a message
//! queue, GCS, MySQL) is a typed addition *here*, not free-form catalog data. In return,
//! a provider cannot declare an incomplete connection (the variant's fields are
//! mandatory), and a consumer binds to typed [`ConnectionField`]s rather than re-spelling
//! coordinate names by hand — so the runtime "does this provider render every required
//! coordinate?" check the old open model needed is gone.
//!
//! # Templates and resolution
//!
//! A provider declares a [`ConnectionTemplate`]: a [`Connection`] whose string fields may
//! contain the `{name}` placeholder. The planner [`resolve`](ConnectionTemplate::resolve)s
//! it per demand, substituting `{name}` with the demanded resource name. The remaining values
//! are concrete (e.g. a relational URL embeds the configured credential directly), so a
//! consumer binds a fully-resolved coordinate rather than deferring it to a compose `${VAR}`.

use serde::{Deserialize, Serialize};

/// A fully-resolved, typed connection to a provisioned resource.
///
/// One variant per resource *flavour*. Every field is a final, concrete string value — the
/// planner has already substituted `{name}`, and credentials/coordinates are wired in directly
/// rather than deferred to a compose `${VAR}`.
///
/// `#[non_exhaustive]`: a future flavour (a message queue, GCS, …) can be added without
/// breaking downstream `match`es.
///
/// `Debug` is **hand-written to redact** the relational password and the URL that embeds it
/// (the object-store credential redacts itself), so a connection never leaks a secret through
/// `{:?}`, `tracing`, or a panic message.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
#[non_exhaustive]
pub enum Connection {
    /// An S3/Blob-style object store. Addressing (`uri`/`bucket`/`endpoint`) is
    /// flavour-independent; the [`credential`](Connection::ObjectStore::credential) carries
    /// the flavour-specific auth.
    ObjectStore {
        /// Client-addressable URI for the resource (`s3://{name}`, `wasbs://…`).
        uri: String,
        /// The bucket/container name.
        bucket: String,
        /// The in-network service endpoint (`http://rustfs:9000`).
        endpoint: String,
        /// The credential needed to authenticate to the store.
        credential: ObjectStoreCredential,
        /// How a consumer trusts the TLS the store is reached over at its *cloud* hostnames,
        /// when the gateway emulates them (see
        /// [`Provides::impersonated_hosts`](crate::Provides::impersonated_hosts)). `None` for a
        /// store reached only at its plain [`endpoint`](Connection::ObjectStore::endpoint).
        /// The planner fills this in; a provider's template leaves it unset. Boxed so the
        /// rarely-set trust doesn't inflate every [`Connection`].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tls_trust: Option<Box<TlsTrust>>,
    },
    /// A relational database, as one URL (with the credential embedded, as libpq-style clients
    /// take it) and as typed parts for clients that take them separately (a JDBC URL plus a
    /// username and password, e.g. Hibernate). Build one with [`Connection::postgres`], which
    /// derives the URL from the parts so the two never disagree.
    RelationalDb {
        /// The full connection URL, e.g. `postgresql://user:pass@db:5432/{name}`.
        url: String,
        /// The server's host (its compose DNS name, e.g. `db`).
        host: String,
        /// The server's port.
        port: u16,
        /// The database name (typically `{name}` in a template).
        database: String,
        /// The role to connect as.
        username: String,
        /// The role's password.
        password: String,
    },
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Connection::ObjectStore {
                uri,
                bucket,
                endpoint,
                credential,
                tls_trust,
            } => f
                .debug_struct("ObjectStore")
                .field("uri", uri)
                .field("bucket", bucket)
                .field("endpoint", endpoint)
                .field("credential", credential)
                .field("tls_trust", tls_trust)
                .finish(),
            Connection::RelationalDb {
                host,
                port,
                database,
                username,
                ..
            } => f
                .debug_struct("RelationalDb")
                .field("url", &"<redacted>")
                .field("host", host)
                .field("port", port)
                .field("database", database)
                .field("username", username)
                .field("password", &"<redacted>")
                .finish(),
        }
    }
}

/// The credential for an [`ObjectStore`](Connection::ObjectStore) — closed per flavour.
///
/// `#[non_exhaustive]` so a future object-store flavour (GCS, …) is not a breaking change
/// for downstream `match`es.
///
/// `Debug` is **hand-written to redact** the secret-bearing fields (the secret access key
/// and the Azure connection string, plus the access key id, itself a sensitive
/// identifier): these are long-lived secrets that must never leak through `{:?}`,
/// `tracing`, or a panic message. `Connection`'s derived `Debug` defers to this impl, so a
/// whole plan can be logged without exposing them. (Mirrors the `AwsCredential` convention
/// in `olai-http`.) The non-secret `region` stays visible.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "flavour")]
#[non_exhaustive]
pub enum ObjectStoreCredential {
    /// S3-style static credentials.
    S3 {
        /// `AWS_ACCESS_KEY_ID`.
        access_key_id: String,
        /// `AWS_SECRET_ACCESS_KEY`.
        secret_access_key: String,
        /// The default region.
        region: String,
        /// The role a consumer may `AssumeRole` into to vend scoped, temporary credentials
        /// (an IAM role ARN). `None` when the store offers no STS endpoint, so consumers can
        /// only use the static keys above.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        role_arn: Option<String>,
    },
    /// An Azure Blob connection string (carries account name + key + endpoint).
    AzureBlob {
        /// The full `AZURE_STORAGE_CONNECTION_STRING` value.
        connection_string: String,
    },
}

impl std::fmt::Debug for ObjectStoreCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ObjectStoreCredential::S3 {
                region, role_arn, ..
            } => f
                .debug_struct("S3")
                .field("access_key_id", &"<redacted>")
                .field("secret_access_key", &"<redacted>")
                .field("region", region)
                .field("role_arn", role_arn)
                .finish(),
            ObjectStoreCredential::AzureBlob { .. } => f
                .debug_struct("AzureBlob")
                .field("connection_string", &"<redacted>")
                .finish(),
        }
    }
}

/// How a consumer trusts a locally-minted certificate authority: the compose volume holding
/// the CA material, where to mount it, and the files inside it each runtime reads.
///
/// Present on a [`Connection::ObjectStore`] whose cloud hostnames the gateway emulates over
/// TLS. A consumer's template mounts [`volume`](Self::volume) at
/// [`mount_path`](Self::mount_path) and points its runtime at the matching file — a JVM at
/// [`jvm_truststore`](Self::jvm_truststore), `botocore` / `object_store` at
/// [`ca_pem`](Self::ca_pem). None of these are secrets (the truststore password is the
/// well-known JDK default), so `Debug` is derived.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TlsTrust {
    /// The compose named volume the CA material lives in.
    pub volume: String,
    /// Where a consumer mounts [`volume`](Self::volume) (read-only).
    pub mount_path: String,
    /// The PEM CA certificate, inside the mount (`SSL_CERT_FILE`, `AWS_CA_BUNDLE`).
    pub ca_pem: String,
    /// A PKCS#12 JVM truststore (the JDK defaults plus the CA), inside the mount.
    pub jvm_truststore: String,
    /// The [`jvm_truststore`](Self::jvm_truststore) password.
    pub jvm_truststore_password: String,
}

/// A provider's connection *template*: a [`Connection`] whose string fields may contain
/// the `{name}` placeholder, substituted per demand by the planner.
///
/// Stored on a provider module's
/// [`Provides::resource_kinds`](crate::Provides::resource_kinds), keyed by the role string.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ConnectionTemplate(pub Connection);

impl ConnectionTemplate {
    /// Resolve this template to a concrete [`Connection`] for the resource named `name`,
    /// substituting every `{name}` placeholder. Only `{name}` is a template hole; all other
    /// values are already concrete.
    pub fn resolve(&self, name: &str) -> Connection {
        let sub = |s: &str| s.replace("{name}", name);
        match &self.0 {
            Connection::ObjectStore {
                uri,
                bucket,
                endpoint,
                credential,
                tls_trust,
            } => Connection::ObjectStore {
                uri: sub(uri),
                bucket: sub(bucket),
                endpoint: sub(endpoint),
                credential: credential.resolve(name),
                tls_trust: tls_trust.clone(),
            },
            Connection::RelationalDb {
                url,
                host,
                port,
                database,
                username,
                password,
            } => Connection::RelationalDb {
                url: sub(url),
                host: sub(host),
                port: *port,
                database: sub(database),
                username: sub(username),
                password: sub(password),
            },
        }
    }
}

impl ObjectStoreCredential {
    /// `AWS_ACCESS_KEY_ID` — the env var an S3 credential's access key id lands under.
    pub const AWS_ACCESS_KEY_ID: &'static str = "AWS_ACCESS_KEY_ID";
    /// `AWS_SECRET_ACCESS_KEY` — the env var an S3 credential's secret lands under.
    pub const AWS_SECRET_ACCESS_KEY: &'static str = "AWS_SECRET_ACCESS_KEY";
    /// `AWS_DEFAULT_REGION` — the env var an S3 credential's region lands under.
    pub const AWS_DEFAULT_REGION: &'static str = "AWS_DEFAULT_REGION";
    /// `AZURE_STORAGE_CONNECTION_STRING` — the env var an Azure credential lands under.
    pub const AZURE_STORAGE_CONNECTION_STRING: &'static str = "AZURE_STORAGE_CONNECTION_STRING";

    /// The conventional `(env-var, value)` pairs an SDK reads to authenticate to a store of
    /// this flavour — `AWS_*` for [`S3`](ObjectStoreCredential::S3),
    /// `AZURE_STORAGE_CONNECTION_STRING` for [`AzureBlob`](ObjectStoreCredential::AzureBlob).
    ///
    /// The planner folds these into `.env` for the chosen object-store provider, so the
    /// typed credential is the single source for both the values a [`RenderSpec`] fragment
    /// reads and the conventional SDK env vars — no hand-listing, no drift.
    ///
    /// [`RenderSpec`]: crate::RenderSpec
    pub fn standard_env(&self) -> Vec<(&'static str, String)> {
        match self {
            ObjectStoreCredential::S3 {
                access_key_id,
                secret_access_key,
                region,
                ..
            } => vec![
                (Self::AWS_ACCESS_KEY_ID, access_key_id.clone()),
                (Self::AWS_SECRET_ACCESS_KEY, secret_access_key.clone()),
                (Self::AWS_DEFAULT_REGION, region.clone()),
            ],
            ObjectStoreCredential::AzureBlob { connection_string } => vec![(
                Self::AZURE_STORAGE_CONNECTION_STRING,
                connection_string.clone(),
            )],
        }
    }

    /// Resolve `{name}` in every field. Credential values rarely template on `{name}`, but
    /// resolving uniformly keeps [`ConnectionTemplate::resolve`] total.
    fn resolve(&self, name: &str) -> ObjectStoreCredential {
        let sub = |s: &str| s.replace("{name}", name);
        match self {
            ObjectStoreCredential::S3 {
                access_key_id,
                secret_access_key,
                region,
                role_arn,
            } => ObjectStoreCredential::S3 {
                access_key_id: sub(access_key_id),
                secret_access_key: sub(secret_access_key),
                region: sub(region),
                role_arn: role_arn.as_deref().map(sub),
            },
            ObjectStoreCredential::AzureBlob { connection_string } => {
                ObjectStoreCredential::AzureBlob {
                    connection_string: sub(connection_string),
                }
            }
        }
    }
}

/// One typed part of a resolved [`Connection`] a demand can bind to an environment
/// variable. The typed replacement for the old stringly coordinate name.
///
/// Not every field is present on every variant — [`Connection::field`] returns `None` for a
/// field the variant lacks (e.g. [`Url`](ConnectionField::Url) on an object store), which
/// the planner surfaces as [`PlanError::UnboundConnectionField`](crate::PlanError::UnboundConnectionField).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionField {
    /// [`Connection::ObjectStore::uri`].
    Uri,
    /// [`Connection::ObjectStore::bucket`].
    Bucket,
    /// [`Connection::ObjectStore::endpoint`].
    Endpoint,
    /// [`ObjectStoreCredential::S3::access_key_id`].
    AccessKeyId,
    /// [`ObjectStoreCredential::S3::secret_access_key`].
    SecretAccessKey,
    /// [`ObjectStoreCredential::S3::region`].
    Region,
    /// [`ObjectStoreCredential::AzureBlob::connection_string`].
    ConnectionString,
    /// [`Connection::RelationalDb::url`].
    Url,
}

impl Connection {
    /// A PostgreSQL [`RelationalDb`](Connection::RelationalDb) from its parts, with the URL
    /// derived as `postgresql://{username}:{password}@{host}:{port}/{database}`. Any part may
    /// hold the `{name}` placeholder when building a [`ConnectionTemplate`].
    pub fn postgres(
        host: impl Into<String>,
        port: u16,
        database: impl Into<String>,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Connection {
        let (host, database, username, password) = (
            host.into(),
            database.into(),
            username.into(),
            password.into(),
        );
        Connection::RelationalDb {
            url: format!("postgresql://{username}:{password}@{host}:{port}/{database}"),
            host,
            port,
            database,
            username,
            password,
        }
    }

    /// The value of one typed [`ConnectionField`], if this connection variant has it.
    ///
    /// Returns `None` for a field absent from the variant (e.g.
    /// [`Url`](ConnectionField::Url) on an object store, or an S3 credential field on an
    /// Azure-backed store).
    pub fn field(&self, field: ConnectionField) -> Option<&str> {
        use ConnectionField as F;
        match (self, field) {
            (Connection::ObjectStore { uri, .. }, F::Uri) => Some(uri),
            (Connection::ObjectStore { bucket, .. }, F::Bucket) => Some(bucket),
            (Connection::ObjectStore { endpoint, .. }, F::Endpoint) => Some(endpoint),
            (Connection::ObjectStore { credential, .. }, _) => credential.field(field),
            (Connection::RelationalDb { url, .. }, F::Url) => Some(url),
            _ => None,
        }
    }

    /// The conventional `(env-var, value)` pairs a provider of this connection contributes
    /// to `.env` so an SDK can authenticate — the object store's
    /// [`ObjectStoreCredential::standard_env`]. A [`RelationalDb`](Connection::RelationalDb)
    /// has none: its credential travels with the connection a consumer binds, not a
    /// stack-wide env var.
    pub fn standard_env(&self) -> Vec<(&'static str, String)> {
        match self {
            Connection::ObjectStore { credential, .. } => credential.standard_env(),
            Connection::RelationalDb { .. } => Vec::new(),
        }
    }
}

impl ObjectStoreCredential {
    /// The value of a credential field, if this credential flavour has it.
    fn field(&self, field: ConnectionField) -> Option<&str> {
        use ConnectionField as F;
        match (self, field) {
            (ObjectStoreCredential::S3 { access_key_id, .. }, F::AccessKeyId) => {
                Some(access_key_id)
            }
            (
                ObjectStoreCredential::S3 {
                    secret_access_key, ..
                },
                F::SecretAccessKey,
            ) => Some(secret_access_key),
            (ObjectStoreCredential::S3 { region, .. }, F::Region) => Some(region),
            (ObjectStoreCredential::AzureBlob { connection_string }, F::ConnectionString) => {
                Some(connection_string)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relational_template_substitutes_name_into_concrete_url() {
        let t = ConnectionTemplate(Connection::postgres(
            "db", 5432, "{name}", "postgres", "postgres",
        ));
        let c = t.resolve("appdb");
        assert_eq!(
            c.field(ConnectionField::Url),
            Some("postgresql://postgres:postgres@db:5432/appdb"),
            "{{name}} is substituted into an otherwise-concrete URL"
        );
        // The typed parts resolve in lock-step with the URL.
        match c {
            Connection::RelationalDb {
                host,
                port,
                database,
                username,
                password,
                ..
            } => {
                assert_eq!(
                    (host.as_str(), port, database.as_str()),
                    ("db", 5432, "appdb")
                );
                assert_eq!(
                    (username.as_str(), password.as_str()),
                    ("postgres", "postgres")
                );
            }
            other => panic!("expected a relational connection, got {other:?}"),
        }
    }

    #[test]
    fn object_store_template_resolves_every_field() {
        let t = ConnectionTemplate(Connection::ObjectStore {
            uri: "s3://{name}".into(),
            bucket: "{name}".into(),
            endpoint: "http://rustfs:9000".into(),
            credential: ObjectStoreCredential::S3 {
                access_key_id: "rustfs".into(),
                secret_access_key: "rustfs".into(),
                region: "us-east-1".into(),
                role_arn: Some("arn:aws:iam::000000000000:role/{name}".into()),
            },
            tls_trust: None,
        });
        let c = t.resolve("artifacts");
        assert_eq!(c.field(ConnectionField::Uri), Some("s3://artifacts"));
        assert_eq!(c.field(ConnectionField::Bucket), Some("artifacts"));
        assert_eq!(
            c.field(ConnectionField::Endpoint),
            Some("http://rustfs:9000")
        );
        assert_eq!(c.field(ConnectionField::AccessKeyId), Some("rustfs"));
        // `{name}` is substituted into the role ARN too.
        assert!(matches!(
            c,
            Connection::ObjectStore {
                credential: ObjectStoreCredential::S3 { role_arn: Some(ref arn), .. },
                ..
            } if arn == "arn:aws:iam::000000000000:role/artifacts"
        ));
        assert_eq!(c.field(ConnectionField::Region), Some("us-east-1"));
        // A field the variant lacks resolves to None.
        assert_eq!(c.field(ConnectionField::Url), None);
        assert_eq!(c.field(ConnectionField::ConnectionString), None);
    }

    #[test]
    fn azure_credential_exposes_connection_string_only() {
        let c = Connection::ObjectStore {
            uri: "wasbs://data@acct".into(),
            bucket: "data".into(),
            endpoint: "http://azurite:10000".into(),
            credential: ObjectStoreCredential::AzureBlob {
                connection_string: "Conn=string".into(),
            },
            tls_trust: None,
        };
        assert_eq!(
            c.field(ConnectionField::ConnectionString),
            Some("Conn=string")
        );
        // S3-only fields are absent under an Azure credential.
        assert_eq!(c.field(ConnectionField::AccessKeyId), None);
    }

    #[test]
    fn standard_env_derives_the_conventional_sdk_vars() {
        let s3 = Connection::ObjectStore {
            uri: "s3://b".into(),
            bucket: "b".into(),
            endpoint: "http://s:1".into(),
            credential: ObjectStoreCredential::S3 {
                access_key_id: "ak".into(),
                secret_access_key: "sk".into(),
                region: "us-east-1".into(),
                role_arn: None,
            },
            tls_trust: None,
        };
        assert_eq!(
            s3.standard_env(),
            vec![
                ("AWS_ACCESS_KEY_ID", "ak".to_string()),
                ("AWS_SECRET_ACCESS_KEY", "sk".to_string()),
                ("AWS_DEFAULT_REGION", "us-east-1".to_string()),
            ]
        );

        let azure = Connection::ObjectStore {
            uri: "wasbs://b@a".into(),
            bucket: "b".into(),
            endpoint: "http://a:1".into(),
            credential: ObjectStoreCredential::AzureBlob {
                connection_string: "Conn=x".into(),
            },
            tls_trust: None,
        };
        assert_eq!(
            azure.standard_env(),
            vec![("AZURE_STORAGE_CONNECTION_STRING", "Conn=x".to_string())]
        );

        // A relational connection contributes no provider-side env vars (its credential is
        // embedded in the URL a consumer binds).
        let db = Connection::postgres("db", 5432, "x", "u", "p");
        assert!(db.standard_env().is_empty());
    }

    #[test]
    fn debug_redacts_the_relational_password_and_url() {
        let db = Connection::postgres("db", 5432, "app", "admin", "hunter2");
        let rendered = format!("{db:?}");
        assert!(!rendered.contains("hunter2"), "{rendered}");
        assert!(
            rendered.contains("<redacted>") && rendered.contains("admin"),
            "{rendered}"
        );
    }

    #[test]
    fn debug_redacts_secret_credential_fields() {
        let s3 = ObjectStoreCredential::S3 {
            access_key_id: "AKIAEXAMPLE".into(),
            secret_access_key: "topsecret".into(),
            region: "us-east-1".into(),
            role_arn: None,
        };
        let rendered = format!("{s3:?}");
        assert!(
            !rendered.contains("AKIAEXAMPLE"),
            "access key id leaked: {rendered}"
        );
        assert!(!rendered.contains("topsecret"), "secret leaked: {rendered}");
        assert!(rendered.contains("<redacted>"));
        // The non-secret region stays visible for observability.
        assert!(
            rendered.contains("us-east-1"),
            "region should stay visible: {rendered}"
        );

        let azure = ObjectStoreCredential::AzureBlob {
            connection_string: "AccountKey=supersecret==".into(),
        };
        let rendered = format!("{azure:?}");
        assert!(
            !rendered.contains("supersecret"),
            "conn string leaked: {rendered}"
        );
        assert!(rendered.contains("<redacted>"));

        // And the secret is hidden when the credential is nested inside a Connection's Debug.
        let conn = Connection::ObjectStore {
            uri: "s3://b".into(),
            bucket: "b".into(),
            endpoint: "http://s:1".into(),
            credential: s3,
            tls_trust: None,
        };
        assert!(!format!("{conn:?}").contains("topsecret"));
    }
}
